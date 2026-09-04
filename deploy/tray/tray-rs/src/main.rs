//! walgit-tray — walgit 服务的系统托盘(macOS / Windows / Linux 一套代码)。
//!
//! 菜单:状态行 · 启动/停止 · 版本升级状态行(abb 同款状态机) ·
//!      打开 Web UI · 退出托盘(服务保持运行)。
//! 升级语义:自动的只有「检测」(每 30 分钟 fetch 比对,启动 30 秒先查一次);
//!      发现新版本只把菜单行变成「⬆️ 升级到新版本」,**由用户点击才升级**:
//!      ff-merge main → cargo 构建 → 备份 → 停 → 热换 → 健康验证,失败回滚。
//!
//! 服务控制:macOS shell 到 walgit-ensure(screen 保活);Windows/Linux
//!          部署目录下的 walgit(.exe) 分离进程 + pidfile。
//! 健康检查:内置裸 HTTP(loopback),零额外依赖。
//! 打开 Web UI:直接开新页面(三平台一致)。

// Windows release 不带控制台:双击静默驻留托盘(debug 构建保留控制台便于排查)。
// 注意 start 引号:cmd 只认双引号,`start "" "url"` 的 "" 是占位标题。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};

const HEALTH_HOST: &str = "127.0.0.1:8081";

// 升级状态机(abb app.slint 同款)
const ST_IDLE: u8 = 0; // 未查:检查更新…
const ST_CHECKING: u8 = 1; // 正在检查更新…
const ST_LATEST: u8 = 2; // 已是最新 ✓(点击重查)
const ST_AVAILABLE: u8 = 3; // ⬆️ 升级到新版本
const ST_INSTALLING: u8 = 4; // 安装中…
const ST_FAILED: u8 = 5; // 失败(点击重查)

fn home() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        PathBuf::from(std::env::var("USERPROFILE").unwrap_or_default())
    }
    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from(std::env::var("HOME").unwrap_or_default())
    }
}

fn deploy_dir() -> PathBuf {
    home().join("walgit")
}

fn repo_dir() -> PathBuf {
    std::env::var("WALGIT_REPO").map(PathBuf::from).unwrap_or_else(|_| {
        #[cfg(target_os = "macos")]
        {
            PathBuf::from("/Volumes/Workspace/GitHub/walgit")
        }
        #[cfg(not(target_os = "macos"))]
        {
            home().join("walgit-repo")
        }
    })
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn pid_file() -> PathBuf {
    deploy_dir().join("walgit.pid")
}

fn exe_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "walgit.exe"
    } else {
        "walgit"
    }
}

fn log_line(s: &str) {
    // 部署目录首次运行可能不存在:建出来,否则日志被 OpenOptions 静默丢弃。
    let _ = std::fs::create_dir_all(deploy_dir());
    let path = deploy_dir().join("tray.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "[{s}]");
    }
}

// ---------- 裸 HTTP ----------

/// 返回 healthz 响应体(含 version 字段);服务不在时 None。
fn healthz() -> Option<String> {
    let mut stream = TcpStream::connect(HEALTH_HOST).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let host = HEALTH_HOST.split(':').next()?;
    write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .ok()?;
    let mut buf = String::new();
    stream.read_to_string(&mut buf).ok()?;
    let body = buf.split_once("\r\n\r\n")?.1;
    body.contains("ok").then(|| body.trim().to_string())
}

/// 从 healthz 响应体提取版本串(short sha)。
#[allow(dead_code)] // 供状态行显示;macOS 上由 Swift 版承担
fn version_of(body: &str) -> String {
    body.split("\"version\":\"")
        .nth(1)
        .map(|rest| rest.split('"').next().unwrap_or("").to_string())
        .unwrap_or_default()
}

// ---------- shell ----------

#[cfg(not(target_os = "windows"))]
fn sh(cmd: &str) -> (i32, String) {
    match std::process::Command::new("sh").arg("-lc").arg(cmd).output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr),
        ),
        Err(e) => (-1, format!("{e}")),
    }
}

#[cfg(target_os = "windows")]
fn sh(cmd: &str) -> (i32, String) {
    match std::process::Command::new("cmd").args(["/C", cmd]).output() {
        Ok(o) => (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).to_string()
                + &String::from_utf8_lossy(&o.stderr),
        ),
        Err(e) => (-1, format!("{e}")),
    }
}

// ---------- 服务控制 ----------

#[cfg(target_os = "macos")]
fn ensure_path() -> PathBuf {
    let candidates = [
        home().join(".claude/skills/walgit/scripts/walgit-ensure"),
        deploy_dir().join("walgit-ensure"),
    ];
    candidates
        .iter()
        .find(|p| p.exists())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

fn service_start() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (code, out) = sh(&format!("'{}' 2>&1", ensure_path().display()));
        if code == 0 {
            Ok(())
        } else {
            Err(out)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let exe = deploy_dir().join(exe_name());
        let cfg = deploy_dir().join("walgit.toml");
        let mut cmd = std::process::Command::new(&exe);
        cmd.arg("serve").arg("--config").arg(&cfg);
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0000_0008 | 0x0000_0200); // DETACHED | NEW_GROUP
        }
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            unsafe {
                cmd.pre_exec(|| {
                    libc::setsid();
                    Ok(())
                });
            }
        }
        let child = cmd
            .spawn()
            .map_err(|e| format!("spawn {}: {e}", exe.display()))?;
        std::fs::write(pid_file(), child.id().to_string())
            .map_err(|e| format!("pidfile: {e}"))?;
        Ok(())
    }
}

fn service_stop() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let (code, out) = sh(&format!("'{}' stop 2>&1", ensure_path().display()));
        if code == 0 {
            Ok(())
        } else {
            Err(out)
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let pid: u32 = std::fs::read_to_string(pid_file())
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .ok_or_else(|| "no pidfile".to_string())?;
        let (code, out) = if cfg!(target_os = "windows") {
            sh(&format!("taskkill /PID {pid} /T /F"))
        } else {
            sh(&format!("kill {pid}"))
        };
        if code == 0 {
            Ok(())
        } else {
            Err(out)
        }
    }
}

/// 升级管线(仅由用户点击触发):fetch → ff-merge → 构建 → 备份 → 停 → 换 → 起 → 验证。
fn upgrade_pipeline(report: &dyn Fn(String)) -> Result<String, String> {
    let repo = repo_dir();
    let repo_s = repo.to_string_lossy().to_string();
    let bin = deploy_dir().join(exe_name());

    report("对齐 main…".into());
    sh(&format!("git -C '{repo_s}' fetch origin main"));
    let (mc, mout) = sh(&format!("cd '{repo_s}' && git merge --ff-only origin/main"));
    if mc != 0 {
        log_line(&format!(
            "upgrade: ff-merge FAILED {}",
            &mout[..mout.len().min(200)]
        ));
        return Err("本地 main 无法快进到 origin/main".into());
    }

    report("构建中…".into());
    #[cfg(target_os = "windows")]
    let (bc, bout) = sh(&format!(
        // `set "VAR=val"&&`:带引号的 set 才不会把 && 前的空格并进值里
        "cd /d {repo_s} && set \"RUSTUP_TOOLCHAIN=1.98.0\"&& cargo build --release -p walgit-cli"
    ));
    #[cfg(not(target_os = "windows"))]
    let (bc, bout) = sh(&format!(
        "cd '{repo_s}' && RUSTUP_TOOLCHAIN=1.98.0 cargo build --release -p walgit-cli"
    ));
    if bc != 0 {
        log_line(&format!(
            "upgrade: build FAILED {}",
            &bout[..bout.len().min(300)]
        ));
        return Err("cargo 构建失败,旧版本继续运行".into());
    }
    let (_, sha_out) = sh(&format!("cd '{repo_s}' && git rev-parse --short=7 HEAD"));
    let sha = sha_out.trim().to_string();

    report("换装中…".into());
    let bak = deploy_dir().join("walgit.bak-tray");
    let _ = std::fs::copy(&bin, &bak);
    let _ = service_stop();
    std::fs::copy(repo.join("target/release").join(exe_name()), &bin)
        .map_err(|e| format!("copy binary: {e}"))?;
    service_start()?;

    for _ in 0..15 {
        if let Some(h) = healthz() {
            if h.contains(&sha) {
                log_line(&format!("upgrade: success {sha}"));
                return Ok(sha);
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    log_line("upgrade: health FAIL — rollback");
    let _ = std::fs::copy(&bak, &bin);
    let _ = service_stop();
    let _ = service_start();
    Err("健康检查未过,已回滚旧版本".into())
}

// ---------- 打开 Web UI(不重复开页) ----------

/// 打开 Web UI:直接开新页面(三平台一致)。
fn open_web() {
    log_line("web: opening page");
    let url = "http://127.0.0.1:8081/";
    #[cfg(target_os = "macos")]
    sh(&format!("open '{url}'"));
    #[cfg(target_os = "windows")]
    sh(&format!("start \"\" \"{url}\"")); // "" 是占位标题;cmd 不认单引号
    #[cfg(all(unix, not(target_os = "macos")))]
    sh(&format!("xdg-open '{url}'"));
}

// ---------- 图标:分支汇入桶(手工光栅化,分状态着色) ----------

fn seg_dist(px: f32, py: f32, p0: [f32; 2], p1: [f32; 2]) -> f32 {
    let (vx, vy) = (p1[0] - p0[0], p1[1] - p0[1]);
    let (wx, wy) = (px - p0[0], py - p0[1]);
    let len2 = vx * vx + vy * vy;
    let t = if len2 == 0.0 {
        0.0
    } else {
        ((wx * vx + wy * vy) / len2).clamp(0.0, 1.0)
    };
    let (dx, dy) = (px - (p0[0] + t * vx), py - (p0[1] + t * vy));
    (dx * dx + dy * dy).sqrt()
}

/// 状态色:运行绿 / 升级橙 / 检查与切换黄 / 停止灰。
fn state_color(running: bool, busy: u8) -> [u8; 3] {
    if busy == 2 {
        [232, 160, 32] // 橙:升级中
    } else if busy == 1 {
        [255, 204, 0] // 黄:切换中
    } else if running {
        [46, 160, 67] // 绿:运行中
    } else {
        [138, 138, 138] // 灰:已停止
    }
}

fn icon_rgba(size: usize, color: [u8; 3]) -> Vec<u8> {
    let n = size;
    let s = 256.0 / n as f32;
    let mut rgba = vec![0u8; n * n * 4];
    let mut segments: Vec<([f32; 2], [f32; 2])> = Vec::new();
    segments.push(([78.0, 180.0], [78.0, 88.0]));
    let mut prev = [186.0_f32, 180.0];
    for i in 1..=24 {
        let t = i as f32 / 24.0;
        let mt = 1.0 - t;
        let x = mt * mt * mt * 186.0
            + 3.0 * mt * mt * t * 186.0
            + 3.0 * mt * t * t * 78.0
            + t * t * t * 78.0;
        let y = mt * mt * mt * 180.0
            + 3.0 * mt * mt * t * 152.0
            + 3.0 * mt * t * t * 152.0
            + t * t * t * 138.0;
        segments.push((prev, [x, y]));
        prev = [x, y];
    }
    let circles: [(f32, f32, f32); 2] = [(78.0, 200.0, 17.0), (186.0, 200.0, 17.0)];
    let slabs: [(f32, f32, f32, f32); 2] =
        [(52.0, 22.0, 152.0, 24.0), (52.0, 50.0, 152.0, 24.0)];
    let half = 7.5;

    for py in 0..n {
        for px in 0..n {
            let cx = (px as f32 + 0.5) * s;
            let cy = 256.0 - (py as f32 + 0.5) * s;
            let mut cov = 0.0f32;
            for sy in 0..4u8 {
                for sx in 0..4u8 {
                    let x = cx + (f32::from(sx) - 1.5) / 4.0 * s;
                    let y = cy + (f32::from(sy) - 1.5) / 4.0 * s;
                    let inside = segments
                        .iter()
                        .any(|(p0, p1)| seg_dist(x, y, *p0, *p1) <= half)
                        || circles.iter().any(|(ccx, ccy, r)| {
                            (x - ccx) * (x - ccx) + (y - ccy) * (y - ccy) <= r * r
                        })
                        || slabs.iter().any(|(rx, ry, rw, rh)| {
                            x >= *rx && x <= rx + rw && y >= *ry && y <= ry + rh
                        });
                    if inside {
                        cov += 1.0 / 16.0;
                    }
                }
            }
            let i = (py * n + px) * 4;
            rgba[i] = color[0];
            rgba[i + 1] = color[1];
            rgba[i + 2] = color[2];
            rgba[i + 3] = (cov * 255.0) as u8;
        }
    }
    rgba
}

// ---------- 事件与状态 ----------

#[derive(Debug, Clone)]
enum Msg {
    Status(bool),
    UpdateState(u8),
    Available(String),
    Note(String),
    Busy(u8),
}

struct MenuHandles {
    status: MenuItem,
    toggle: MenuItem,
    upgrade: MenuItem,
}

struct App {
    tray: Option<TrayIcon>,
    items: Option<MenuHandles>,
    proxy: Option<Arc<EventLoopProxy<Msg>>>,
    running: bool,
    busy: u8,  // 0 idle, 1 switching, 2 upgrading
    state: u8, // 升级状态机(ST_*)
    version: String,
    available_sha: String,
    note: String,
}

impl App {
    fn current_version(&self) -> String {
        let v = self.version.clone();
        if v.is_empty() {
            "…".to_string()
        } else {
            v[..7.min(v.len())].to_string()
        }
    }

    fn update_icon(&mut self) {
        let Some(tray) = self.tray.as_mut() else {
            return;
        };
        let color = state_color(self.running, self.busy);
        if let Ok(icon) = tray_icon::Icon::from_rgba(icon_rgba(32, color), 32, 32) {
            let _ = tray.set_icon(Some(icon));
        }
    }

    fn rebuild_menu(&mut self) {
        let Some(h) = &self.items else { return };
        let running = self.running;
        h.status.set_text(if self.busy == 1 {
            "walgit 服务:切换中…".into()
        } else if running {
            format!("walgit 服务:运行中 · {}", self.current_version())
        } else {
            "walgit 服务:已停止".into()
        });
        h.toggle.set_text(
            if self.busy == 1 {
                "切换中…"
            } else if running {
                "停止服务"
            } else {
                "启动服务"
            }
            .to_string(),
        );
        h.toggle.set_enabled(self.busy == 0);
        let cur = self.current_version();
        h.upgrade.set_text(match self.state {
            ST_CHECKING => format!("版本 {cur} · 正在检查更新…"),
            ST_LATEST => format!("版本 {cur} · 已是最新 ✓(点击重查)"),
            ST_AVAILABLE => format!("⬆️ 升级到新版本 {}(当前 {cur})", self.available_sha),
            ST_INSTALLING => {
                if self.note.is_empty() {
                    "升级中…".into()
                } else {
                    format!("升级中… · {}", self.note)
                }
            }
            ST_FAILED => "上次升级失败(点击重查)".into(),
            _ => format!("版本 {cur} · 检查更新…"),
        });
        h.upgrade.set_enabled(
            self.busy == 0 && self.state != ST_CHECKING && self.state != ST_INSTALLING,
        );
    }
}

impl ApplicationHandler<Msg> for App {
    fn resumed(&mut self, _loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _loop: &ActiveEventLoop,
        _window: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn user_event(&mut self, _loop: &ActiveEventLoop, msg: Msg) {
        match msg {
            Msg::Status(up) => {
                self.running = up;
                if up {
                    // 版本串从 healthz 取——轮询线程已带;此处只做占位
                }
            }
            Msg::UpdateState(s) => self.state = s,
            Msg::Available(sha) => self.available_sha = sha,
            Msg::Note(n) => self.note = n,
            Msg::Busy(b) => self.busy = b,
        }
        self.update_icon();
        self.rebuild_menu();
    }

    fn about_to_wait(&mut self, _loop: &ActiveEventLoop) {
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            let id = event.id.as_ref().to_string();
            let Some(proxy) = self.proxy.clone() else {
                continue;
            };
            match id.as_str() {
                "start" | "stop" => {
                    if self.busy == 0 {
                        self.busy = 1;
                        self.rebuild_menu();
                        std::thread::spawn(move || {
                            let r = if id == "start" {
                                service_start()
                            } else {
                                service_stop()
                            };
                            log_line(&format!("{id} -> {r:?}"));
                            let _ = proxy.send_event(Msg::Busy(0));
                            let _ = proxy.send_event(Msg::Status(healthz().is_some()));
                        });
                    }
                }
                "upgrade" => match self.state {
                    ST_IDLE | ST_LATEST | ST_FAILED => {
                        self.state = ST_CHECKING;
                        self.rebuild_menu();
                        std::thread::spawn(move || {
                            let repo = repo_dir();
                            let repo_s = repo.to_string_lossy().to_string();
                            sh(&format!("git -C '{repo_s}' fetch origin main"));
                            let (c1, lout) =
                                sh(&format!("cd '{repo_s}' && git rev-parse HEAD"));
                            let (c2, rout) =
                                sh(&format!("cd '{repo_s}' && git rev-parse origin/main"));
                            let local = lout.trim().to_string();
                            let remote = rout.trim().to_string();
                            if c1 != 0 || c2 != 0 || local.is_empty() || remote.is_empty() {
                                let _ = proxy.send_event(Msg::UpdateState(ST_FAILED));
                            } else if local != remote {
                                let _ = proxy.send_event(Msg::UpdateState(ST_AVAILABLE));
                                let _ = proxy.send_event(Msg::Available(
                                    remote[..7.min(remote.len())].to_string(),
                                ));
                            } else {
                                let _ = proxy.send_event(Msg::UpdateState(ST_LATEST));
                            }
                            log_line(&format!(
                                "detect: local={:.7} remote={:.7}",
                                local, remote
                            ));
                        });
                    }
                    ST_AVAILABLE => {
                        // 用户点击升级:这里才真正跑升级管线
                        self.state = ST_INSTALLING;
                        self.busy = 2;
                        self.rebuild_menu();
                        std::thread::spawn(move || {
                            let r = upgrade_pipeline(&|m| {
                                let _ = proxy.send_event(Msg::Note(m));
                            });
                            let _ = proxy.send_event(Msg::Busy(0));
                            let _ = proxy.send_event(Msg::Note(String::new()));
                            let _ = proxy.send_event(Msg::UpdateState(match &r {
                                Ok(_) => ST_LATEST,
                                Err(_) => ST_FAILED,
                            }));
                            let _ = proxy.send_event(Msg::Status(healthz().is_some()));
                            log_line(&format!("upgrade -> {r:?}"));
                        });
                    }
                    _ => {}
                },
                "web" => open_web(),
                "quit" => std::process::exit(0), // 仅退托盘;服务是独立进程
                _ => {}
            }
            self.rebuild_menu();
        }
    }
}

// ---------- 单实例(Windows) ----------

/// 命名互斥:已有托盘实例在跑则 false。句柄故意泄漏——进程生命周期即持有期。
#[cfg(target_os = "windows")]
fn single_instance_ok() -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name: Vec<u16> = std::ffi::OsStr::new("Local\\walgit-tray")
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
        h != 0 && GetLastError() != ERROR_ALREADY_EXISTS
    }
}

fn main() {
    // 单实例:双击多次不叠图标、不留幽灵进程。
    #[cfg(target_os = "windows")]
    if !single_instance_ok() {
        log_line("second instance — exiting");
        return;
    }
    // GUI 子系统下 panic 无控制台可见,落进 tray.log。
    std::panic::set_hook(Box::new(|info| log_line(&format!("panic: {info}"))));
    log_line("tray-rs launched");
    let event_loop = EventLoop::<Msg>::with_user_event().build().unwrap();
    let proxy = Arc::new(event_loop.create_proxy());

    let icon =
        tray_icon::Icon::from_rgba(icon_rgba(32, state_color(false, 0)), 32, 32).expect("icon rgba");
    let menu = Menu::new();
    let status = MenuItem::with_id("status", "walgit 服务:检查中…", false, None);
    let toggle = MenuItem::with_id("stop", "停止服务", true, None);
    let upgrade = MenuItem::with_id("upgrade", "版本 … · 检查更新…", true, None);
    let web = MenuItem::with_id("web", "打开 Web UI", true, None);
    let quit = MenuItem::with_id("quit", "退出托盘(服务保持运行)", true, None);
    let _ = menu.append_items(&[
        &status,
        &toggle,
        &PredefinedMenuItem::separator(),
        &upgrade,
        &PredefinedMenuItem::separator(),
        &web,
        &PredefinedMenuItem::separator(),
        &quit,
    ]);

    let tray = TrayIconBuilder::new()
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_tooltip("walgit — 仓库活在桶上")
        .build()
        .expect("tray build");

    let mut app = App {
        tray: Some(tray),
        items: Some(MenuHandles {
            status,
            toggle,
            upgrade,
        }),
        proxy: Some(proxy.clone()),
        running: healthz().is_some(),
        busy: 0,
        state: ST_IDLE,
        version: String::new(),
        available_sha: String::new(),
        note: String::new(),
    };
    app.rebuild_menu();

    // 状态轮询线程(5s):status + version
    let poll_proxy = proxy;
    std::thread::spawn(move || loop {
        let body = healthz();
        let up = body.is_some();
        let _ = poll_proxy.send_event(Msg::Status(up));
        std::thread::sleep(Duration::from_secs(5));
    });
    // 自动检测线程(启动 30 秒一次,此后每 30 分钟;静默,失败不打扰)
    let detect_proxy = event_loop.create_proxy();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        loop {
            let repo = repo_dir().to_string_lossy().to_string();
            sh(&format!("git -C '{repo}' fetch origin main"));
            let (c1, lout) = sh(&format!("cd '{repo}' && git rev-parse HEAD"));
            let (c2, rout) = sh(&format!("cd '{repo}' && git rev-parse origin/main"));
            let local = lout.trim().to_string();
            let remote = rout.trim().to_string();
            if c1 == 0 && c2 == 0 && !local.is_empty() && !remote.is_empty() {
                if local != remote {
                    let _ = detect_proxy.send_event(Msg::UpdateState(ST_AVAILABLE));
                    let _ = detect_proxy.send_event(Msg::Available(
                        remote[..7.min(remote.len())].to_string(),
                    ));
                }
                log_line(&format!("detect: local={:.7} remote={:.7}", local, remote));
            } else {
                log_line("detect: skip (git failed)");
            }
            std::thread::sleep(Duration::from_secs(1800));
        }
    });

    event_loop.run_app(&mut app).unwrap();
}
