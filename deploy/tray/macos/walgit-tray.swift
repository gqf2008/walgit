// walgit-tray — macOS 菜单栏托盘,管理本机 walgit 服务
// 功能:启停(walgit-ensure)、新版本自动检测(每 30 分钟 fetch 比对,
//      发现新版本仅在菜单/通知里提示,由用户点击后才升级:
//      ff-merge main → cargo 构建 → 热换 → 健康验证,失败回滚)、
//      退出(仅退出托盘,服务不受影响)。
// 路径约定:部署目录 ~/walgit(二进制 walgit + run-walgit.sh + walgit-ensure),
//          源码仓库默认 /Volumes/Workspace/GitHub/walgit
//          (可用 `defaults write com.walgit.tray repoPath <路径>` 覆盖)。
// 日志:~/walgit/tray.log

import AppKit

let deployDir = NSString(string: "~/walgit").expandingTildeInPath
let healthURL = URL(string: "http://127.0.0.1:8081/healthz")!
let webURL = URL(string: "http://walgit.localhost:8081/")!
let logPath = NSString(string: "~/walgit/tray.log").expandingTildeInPath

func ensurePath() -> String {
    let candidates = [
        NSString(string: "~/.claude/skills/walgit/scripts/walgit-ensure").expandingTildeInPath,
        NSString(string: "~/walgit/walgit-ensure").expandingTildeInPath,
    ]
    return candidates.first { FileManager.default.fileExists(atPath: $0) } ?? candidates[0]
}

func logLine(_ s: String) {
    let line = "[\(DateFormatter.localizedString(from: Date(), dateStyle: .none, timeStyle: .medium))] \(s)\n"
    if let fh = FileHandle(forWritingAtPath: logPath) {
        fh.seekToEndOfFile()
        fh.write(line.data(using: .utf8)!)
        fh.closeFile()
    } else {
        try? line.write(toFile: logPath, atomically: true, encoding: .utf8)
    }
}

/// 跑一条 login-shell 命令(继承用户 PATH:cargo/rustup 等),返回 (code, out)。
func sh(_ command: String) -> (Int32, String) {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/bin/zsh")
    p.arguments = ["-lc", command]
    let pipe = Pipe()
    p.standardOutput = pipe
    p.standardError = pipe
    do {
        try p.run()
    } catch {
        return (-1, "\(error)")
    }
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    return (p.terminationStatus, String(data: data, encoding: .utf8) ?? "")
}

enum UpdateState {
    case idle, checking, latest, available, installing, failed
}

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    var statusItem: NSStatusItem!
    var pollTimer: Timer?
    var autoTimer: Timer?
    var serviceState = "checking"   // running | stopped | checking
    var serviceVersion = ""
    var transitioning = false
    /// 升级菜单状态机(abb 同款):idle 未查 / checking 检查中 / latest 已最新 /
    /// available 有新版本 / installing 安装中 / failed 失败(点击重查)。
    var updateState = UpdateState.idle
    var availableSha = ""
    var busyNote = ""
    var deniedBrowsers = Set<String>()
    var lastReopenOpen = Date.distantPast

    func applicationDidFinishLaunching(_ n: Notification) {
        NSApp.setActivationPolicy(.regular)   // Dock 驻留 + 菜单栏双驻留
        NSApp.applicationIconImage = renderDockIcon(256)
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.image = makeIcon(color: .systemYellow)
        statusItem.button?.title = ""
        let menu = NSMenu()
        menu.delegate = self
        statusItem.menu = menu
        rebuildMenu()
        pollTimer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in self.poll() }
        poll()
        // 启动 30 秒后做一次升级检查;此后每 30 分钟。
        DispatchQueue.global().asyncAfter(deadline: .now() + 30) { self.autoCheck() }
        autoTimer = Timer.scheduledTimer(withTimeInterval: 1800, repeats: true) { _ in
            DispatchQueue.global().async { self.autoCheck() }
        }
        logLine("tray launched")
    }

    /// Dock 图标:深色圆角方 + 绿色品牌标
    func renderDockIcon(_ side: CGFloat) -> NSImage {
        let img = NSImage(size: NSSize(width: side, height: side))
        img.lockFocus()
        let s = side / 256.0
        NSColor(calibratedRed: 0.10, green: 0.11, blue: 0.13, alpha: 1).setFill()
        NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: side, height: side),
                     xRadius: 58*s, yRadius: 58*s).fill()
        let green = NSColor(calibratedRed: 0.18, green: 0.63, blue: 0.26, alpha: 1)
        green.setStroke(); green.setFill()
        func dot(_ cx: CGFloat, _ cy: CGFloat, _ r: CGFloat) {
            NSBezierPath(ovalIn: NSRect(x: (cx-r)*s, y: (cy-r)*s, width: 2*r*s, height: 2*r*s)).fill()
        }
        dot(78, 200, 17); dot(186, 200, 17)
        let p = NSBezierPath()
        p.move(to: NSPoint(x: 78*s, y: 180*s))
        p.line(to: NSPoint(x: 78*s, y: 88*s))
        p.lineWidth = 16*s; p.lineCapStyle = .round; p.stroke()
        let b = NSBezierPath()
        b.move(to: NSPoint(x: 186*s, y: 180*s))
        b.curve(to: NSPoint(x: 78*s, y: 138*s),
                controlPoint1: NSPoint(x: 186*s, y: 152*s),
                controlPoint2: NSPoint(x: 78*s, y: 152*s))
        b.lineWidth = 16*s; b.lineCapStyle = .round; b.stroke()
        for y in [22.0, 50.0] {
            NSBezierPath(roundedRect: NSRect(x: 52*s, y: y*s, width: 152*s, height: 24*s),
                         xRadius: 12*s, yRadius: 12*s).fill()
        }
        img.unlockFocus()
        return img
    }

    /// 点击 Dock 图标 = 打开 Web UI
    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        if !flag {
            let now = Date()
            if now.timeIntervalSince(lastReopenOpen) > 3 {
                lastReopenOpen = now
                openWeb()
            }
        }
        return true
    }

    // MARK: - 状态轮询

    func poll() {
        var req = URLRequest(url: healthURL)
        req.timeoutInterval = 3
        URLSession.shared.dataTask(with: req) { data, resp, _ in
            DispatchQueue.main.async {
                if let http = resp as? HTTPURLResponse, http.statusCode == 200,
                   let d = data,
                   let obj = try? JSONSerialization.jsonObject(with: d) as? [String: Any] {
                    self.serviceState = "running"
                    self.serviceVersion = (obj["version"] as? String) ?? ""
                } else {
                    self.serviceState = "stopped"
                    self.serviceVersion = ""
                }
                self.refreshButton()
            }
        }.resume()
    }

    /// walgit 品牌图标:两个提交点经弧线汇入干线,落入两层「桶」板。
    /// 分状态着色(abb 同款思路):运行绿 / 升级橙 / 切换黄 / 停止灰。
    func makeIcon(color: NSColor) -> NSImage {
        let side: CGFloat = 18
        let img = NSImage(size: NSSize(width: side, height: side))
        img.lockFocus()
        color.setStroke()
        color.setFill()
        let s = side / 256.0
        func dot(_ cx: CGFloat, _ cy: CGFloat, _ r: CGFloat) {
            NSBezierPath(ovalIn: NSRect(x: (cx-r)*s, y: (cy-r)*s, width: 2*r*s, height: 2*r*s)).fill()
        }
        dot(78, 200, 17); dot(186, 200, 17)
        let p = NSBezierPath()
        p.move(to: NSPoint(x: 78*s, y: 180*s))
        p.line(to: NSPoint(x: 78*s, y: 88*s))
        p.lineWidth = 15*s; p.lineCapStyle = .round; p.stroke()
        let b = NSBezierPath()
        b.move(to: NSPoint(x: 186*s, y: 180*s))
        b.curve(to: NSPoint(x: 78*s, y: 138*s),
                controlPoint1: NSPoint(x: 186*s, y: 152*s),
                controlPoint2: NSPoint(x: 78*s, y: 152*s))
        b.lineWidth = 15*s; b.lineCapStyle = .round; b.stroke()
        for y in [22.0, 50.0] {
            NSBezierPath(roundedRect: NSRect(x: 52*s, y: y*s, width: 152*s, height: 24*s),
                         xRadius: 12*s, yRadius: 12*s).fill()
        }
        img.unlockFocus()
        return img
    }

    func refreshButton() {
        let color: NSColor
        if updateState == .installing { color = .systemOrange }
        else if transitioning { color = .systemYellow }
        else if serviceState == "running" { color = .systemGreen }
        else { color = .systemGray }
        statusItem.button?.image = makeIcon(color: color)
        statusItem.button?.title = ""
        rebuildMenu()
    }

    func menuNeedsUpdate(_ menu: NSMenu) { rebuildMenu() }

    func rebuildMenu() {
        let menu = statusItem.menu!
        menu.removeAllItems()

        let stateText: String
        switch serviceState {
        case "running": stateText = "运行中 \(serviceVersion.isEmpty ? "" : "· \(serviceVersion)")"
        case "stopped": stateText = "已停止"
        default: stateText = "检查中…"
        }
        let head = NSMenuItem(title: "walgit 服务:\(stateText)", action: nil, keyEquivalent: "")
        head.isEnabled = false
        menu.addItem(head)

        if transitioning {
            let t = NSMenuItem(title: "切换中…", action: nil, keyEquivalent: "")
            t.isEnabled = false
            menu.addItem(t)
        } else if serviceState == "running" {
            menu.addItem(NSMenuItem(title: "停止服务", action: #selector(stopService), keyEquivalent: "s"))
        } else {
            menu.addItem(NSMenuItem(title: "启动服务", action: #selector(startService), keyEquivalent: "r"))
        }

        menu.addItem(.separator())

        let cur = serviceVersion.isEmpty ? "…" : String(serviceVersion.prefix(7))
        let up: NSMenuItem
        switch updateState {
        case .idle:
            up = NSMenuItem(title: "版本 \(cur) · 检查更新…", action: #selector(checkUpdateNow), keyEquivalent: "")
        case .checking:
            up = NSMenuItem(title: "版本 \(cur) · 正在检查更新…", action: nil, keyEquivalent: "")
            up.isEnabled = false
        case .latest:
            up = NSMenuItem(title: "版本 \(cur) · 已是最新 ✓(点击重查)", action: #selector(checkUpdateNow), keyEquivalent: "")
        case .available:
            up = NSMenuItem(title: "⬆️ 升级到新版本 \(availableSha)(当前 \(cur))", action: #selector(doUpgradeNow), keyEquivalent: "")
        case .installing:
            up = NSMenuItem(title: "升级中…\(busyNote.isEmpty ? "" : " · " + busyNote)", action: nil, keyEquivalent: "")
            up.isEnabled = false
        case .failed:
            up = NSMenuItem(title: "上次升级失败(点击重查)", action: #selector(checkUpdateNow), keyEquivalent: "")
        }
        menu.addItem(up)

        menu.addItem(.separator())
        menu.addItem(NSMenuItem(title: "打开 Web UI", action: #selector(openWeb), keyEquivalent: "w"))
        menu.addItem(.separator())
        let quit = NSMenuItem(title: "退出托盘(服务保持运行)", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.addItem(quit)
    }

    // MARK: - 动作

    @objc func startService() {
        transitioning = true; refreshButton()
        DispatchQueue.global().async {
            let (code, out) = sh("'\(ensurePath())' 2>&1")
            logLine("start rc=\(code): \(out.suffix(200))")
            DispatchQueue.main.async {
                self.transitioning = false
                self.poll()
            }
        }
    }

    @objc func stopService() {
        transitioning = true; refreshButton()
        DispatchQueue.global().async {
            let (code, out) = sh("'\(ensurePath())' stop 2>&1")
            logLine("stop rc=\(code): \(out.suffix(200))")
            DispatchQueue.main.async {
                self.transitioning = false
                self.poll()
            }
        }
    }

    @objc func openWeb() { openWebFocused() }

    /// 打开 Web UI:先在「默认浏览器 + 所有运行中的浏览器」里找 8081 的
    /// 已开标签并激活(原页面必须显示,绝不另开新页);全都没有才新开。
    /// 打开 Web UI:先在「默认浏览器 + 所有运行中的浏览器」里找 8081 的
    /// 已开标签并激活(原页面必须显示,绝不另开新页);全都没有才新开。
    func openWebFocused() {
        DispatchQueue.global().async { [weak self] in
            guard let self else { return }
            let target = ":8081"

            // 候选次序:系统默认浏览器(open 当初就开在它那里)→ 运行中的
            // 浏览器进程(动态匹配,不靠固定名单)→ 常用名单兜底。
            var candidates: [String] = []
            if let def = self.defaultBrowserName() { candidates.append(def) }
            for app in NSWorkspace.shared.runningApplications
            where app.activationPolicy == .regular {
                if let name = app.localizedName { candidates.append(name) }
            }
            candidates.append(contentsOf: ["Google Chrome", "Microsoft Edge", "Safari", "Arc", "Brave Browser"])
            let keys = ["chrome", "chromium", "edge", "brave", "arc", "vivaldi",
                        "opera", "safari", "dia", "comet", "quark", "sigma", "browser"]
            var ordered: [String] = []
            for c in candidates where !c.isEmpty {
                let lc = c.lowercased()
                if keys.contains(where: { lc.contains($0) }), !ordered.contains(c) {
                    ordered.append(c)
                }
            }

            var sawRunningBrowser: String?
            for browser in ordered {
                if deniedBrowsers.contains(browser) { continue }
                let (rc, isRunning) = sh(
                    "osascript -e 'application \"\(browser)\" is running' 2>&1")
                guard rc == 0,
                      isRunning.trimmingCharacters(in: .whitespacesAndNewlines) == "true"
                else { continue }
                sawRunningBrowser = sawRunningBrowser ?? browser

                // Edge 等 Chromium 系存在无标签壳窗,「every tab」会 -1728:
                // 每窗独立 try + count 容错。
                let safari = browser.lowercased() == "safari"
                let focus: String = safari
                    ? "set current tab of w to tab i of w"
                    : "set active tab index of w to i"
                let script = """
                on run argv
                  set target to item 1 of argv
                  set found to "none"
                  tell application "\(browser)"
                    repeat with w in windows
                      try
                        set n to count of tabs of w
                        repeat with i from 1 to n
                          if URL of tab i of w contains target then
                            \(focus)
                            set index of w to 1
                            activate
                            return "found"
                          end if
                        end repeat
                      end try
                    end repeat
                  end tell
                  return found
                end run
                """
                let tmp = NSTemporaryDirectory() + "walgit-focus.applescript"
                try? script.write(toFile: tmp, atomically: true, encoding: .utf8)
                let (oc, oout) = sh("osascript '\(tmp)' '\(target)' 2>&1")
                if oc == 0, oout.contains("found") {
                    logLine("web: focused existing tab in \(browser)")
                    return
                }
                if oout.contains("Not authorized") || oout.contains("(-1743)")
                    || oout.contains("not allowed") {
                    deniedBrowsers.insert(browser)
                    logLine("web: automation denied for \(browser) — 本会话跳过")
                } else {
                    logLine("web: no matching tab in \(browser) (\(oout.suffix(80)))")
                }
            }

            // 终局:默认浏览器还在运行 ⇒ 页面大概率就在它里面,只激活、绝不新开;
            // 真的一个浏览器都没开 ⇒ 才新开页面。
            if let def = defaultBrowserName() {
                let (rc, isRunning) = sh(
                    "osascript -e 'application \"\(def)\" is running' 2>&1")
                if rc == 0,
                   isRunning.trimmingCharacters(in: .whitespacesAndNewlines) == "true" {
                    logLine("web: focus failed but default browser (\(def)) running — activate only")
                    _ = sh("osascript -e 'tell application \"\(def)\" to activate' 2>&1")
                    return
                }
            }
            if let b = sawRunningBrowser {
                logLine("web: focus failed, activate \(b) only")
                _ = sh("osascript -e 'tell application \"\(b)\" to activate' 2>&1")
                return
            }
            logLine("web: no browser running, opening new page")
            DispatchQueue.main.async { NSWorkspace.shared.open(webURL) }
        }
    }

    /// 系统默认浏览器(http handler)的名字,如 "Google Chrome"。
    func defaultBrowserName() -> String? {
        guard let url = URL(string: "http://example.com"),
              let appURL = NSWorkspace.shared.urlForApplication(toOpen: url)
        else { return nil }
        return appURL.lastPathComponent.replacingOccurrences(of: ".app", with: "")
    }

    @objc func checkUpdateNow() {
        updateState = .checking; refreshButton()
        DispatchQueue.global().async {
            let repo = self.repoPath()
            _ = sh("git -C '\(repo)' fetch origin main 2>&1")
            let (c1, lOut) = sh("cd '\(repo)' && git rev-parse HEAD")
            let (c2, rOut) = sh("cd '\(repo)' && git rev-parse origin/main")
            let l = lOut.trimmingCharacters(in: .whitespacesAndNewlines)
            let r = rOut.trimmingCharacters(in: .whitespacesAndNewlines)
            DispatchQueue.main.async {
                guard c1 == 0, c2 == 0, !l.isEmpty, !r.isEmpty else {
                    self.updateState = .failed; self.refreshButton(); return
                }
                if l != r {
                    self.updateState = .available
                    self.availableSha = String(r.prefix(7))
                } else {
                    self.updateState = .latest
                }
                self.refreshButton()
            }
        }
    }

    @objc func doUpgradeNow() {
        updateState = .installing; busyNote = ""; refreshButton()
        DispatchQueue.global().async { self.upgradePipeline() }
    }

    func autoCheck() {
        guard updateState != .installing, updateState != .checking, !transitioning else { return }
        guard updateState == .idle || updateState == .latest || updateState == .failed else { return }
        DispatchQueue.global().async {
            let repo = self.repoPath()
            _ = sh("git -C '\(repo)' fetch origin main 2>&1")
            let (c1, lOut) = sh("cd '\(repo)' && git rev-parse HEAD")
            let (c2, rOut) = sh("cd '\(repo)' && git rev-parse origin/main")
            let l = lOut.trimmingCharacters(in: .whitespacesAndNewlines)
            let r = rOut.trimmingCharacters(in: .whitespacesAndNewlines)
            DispatchQueue.main.async {
                guard c1 == 0, c2 == 0, !l.isEmpty, !r.isEmpty else { return } // 静默:失败不打扰
                if l != r {
                    self.updateState = .available
                    self.availableSha = String(r.prefix(7))
                    self.notify("walgit 发现新版本", self.availableSha + " — 点托盘菜单升级")
                } else if self.updateState == .idle {
                    self.updateState = .latest
                }
                self.refreshButton()
            }
            logLine("detect: \(l != r ? "update available \(r.prefix(7))" : "up to date (\(l.prefix(7)))")")
        }
    }

    func repoPath() -> String {
        let custom = UserDefaults.standard.string(forKey: "repoPath")
        return custom ?? "/Volumes/Workspace/GitHub/walgit"
    }

    /// 升级管线(仅由用户点击触发):fetch → ff-merge main → 构建 → 备份 →
    /// 停 → 换 → 起 → 健康验证,失败回滚。
    func upgradePipeline() {
        defer {
            DispatchQueue.main.async {
                self.refreshButton()
                self.poll()
            }
        }
        let repo = repoPath()
        let bin = "\(deployDir)/walgit"
        let note: (String) -> Void = { m in
            DispatchQueue.main.async { self.busyNote = m; self.refreshButton() }
        }

        // 0. 对齐到 origin/main(不快进就不构建——否则会重建旧版本)。
        note("对齐 main…")
        _ = sh("git -C '\(repo)' fetch origin main 2>&1")
        let (mc, mout) = sh("cd '\(repo)' && git merge --ff-only origin/main 2>&1")
        guard mc == 0 else {
            logLine("upgrade: ff-merge FAILED \(mout.suffix(300))")
            note("main 无法快进(本地有分叉?),见 tray.log")
            notify("walgit 升级中止", "本地 main 无法快进到 origin/main")
            return
        }

        logLine("upgrade: build begin")
        note("构建中…")
        let (bc, bout) = sh("cd '\(repo)' && RUSTUP_TOOLCHAIN=1.98.0 cargo build --release -p walgit-cli 2>&1")
        guard bc == 0 else {
            logLine("upgrade: build FAILED \(bout.suffix(400))")
            note("构建失败(见 tray.log)")
            notify("walgit 升级失败", "cargo 构建失败,旧版本继续运行")
            return
        }
        let (_, shaOut) = sh("cd '\(repo)' && git rev-parse --short=7 HEAD")
        let sha = shaOut.trimmingCharacters(in: .whitespacesAndNewlines)

        note("换装中…")
        _ = sh("cp '\(bin)' '\(bin).bak-tray'")
        let (sc, sout) = sh("'\(ensurePath())' stop 2>&1")
        logLine("upgrade: stop rc=\(sc) \(sout.suffix(120))")
        _ = sh("cp '\(repo)/target/release/walgit' '\(bin)'")
        let (rc, rout) = sh("'\(ensurePath())' 2>&1")
        logLine("upgrade: start rc=\(rc) \(rout.suffix(120))")

        // 健康验证 ≤15s,失败回滚
        var ok = false
        for _ in 0..<15 {
            let (hc, hout) = sh("curl -sf --max-time 2 http://127.0.0.1:8081/healthz 2>&1 || true")
            if hc == 0, hout.contains("ok"), hout.contains(sha) { ok = true; break }
            sleep(1)
        }
        if ok {
            logLine("upgrade: success \(sha)")
            DispatchQueue.main.async {
                self.updateState = .latest
                self.refreshButton()
            }
            notify("walgit 已升级", "版本 \(sha),服务已重启")
        } else {
            logLine("upgrade: health FAIL — rollback")
            _ = sh("cp '\(bin).bak-tray' '\(bin)'")
            _ = sh("'\(ensurePath())' stop 2>&1; '\(ensurePath())' 2>&1")
            DispatchQueue.main.async { self.updateState = .failed }
            notify("walgit 升级失败", "健康检查未过,已回滚旧版本")
        }
    }

    func notify(_ title: String, _ body: String) {
        let t = title.replacingOccurrences(of: "\"", with: "'")
        let b = body.replacingOccurrences(of: "\"", with: "'")
        _ = sh("osascript -e 'display notification \"\(b)\" with title \"\(t)\"' 2>&1")
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.run()
