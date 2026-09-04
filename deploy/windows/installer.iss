; deploy/windows/installer.iss — walgit Windows 安装程序(Inno Setup 6)。
;
; 构建(仓库根,先备好二进制):
;   cargo build --release --bin walgit
;   cargo build --release --manifest-path deploy/tray/tray-rs/Cargo.toml
;   ISCC -DMyAppVersion=0.1.0 deploy\windows\installer.iss
; 产物:deploy/windows/Output/walgit-setup-<version>-x64.exe
; CI(release.yml 的 windows leg)以 tag 传版本。形态见 deploy/windows/README.md。

#ifndef MyAppVersion
#define MyAppVersion "0.0.0-dev"
#endif

#define MyAppName "walgit"
#define MyAppExeName "walgit-tray.exe"

[Setup]
AppId={{B4776A83-9C52-4A9E-8F1D-0A5F3E2D1C74}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=walgit
; 部署目录与 tray-rs 的 deploy_dir() 约定一致:%USERPROFILE%\walgit
DefaultDirName={%USERPROFILE}\walgit
AppendDefaultDirName=no
DirExistsWarning=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; 托盘/服务是我们自己 taskkill 的(见 [Code]),不要弹重启管理器提示
CloseApplications=no
OutputDir=Output
OutputBaseFilename=walgit-setup-{#MyAppVersion}-x64
UninstallDisplayName={#MyAppName}
UninstallDisplayIcon={app}\{#MyAppExeName}
MinVersion=10.0

[Languages]
; 中文语言包入库(官方 unofficial 翻译,Inno 不随基包分发);引用相对脚本目录
Name: "chinesesimplified"; MessagesFile: "ChineseSimplified.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
chinesesimplified.AutoStartTask=开机自动启动 walgit 托盘(&A)
english.AutoStartTask=Start the walgit tray at logon (&A)
chinesesimplified.DesktopIconTask=创建桌面快捷方式(&D)
english.DesktopIconTask=Create a desktop shortcut (&D)
chinesesimplified.LaunchTray=启动 walgit 托盘(服务请在托盘菜单「启动服务」)
english.LaunchTray=Launch the walgit tray (start the service from its menu)

[Tasks]
Name: "autostart"; Description: "{cm:AutoStartTask}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "desktopicon"; Description: "{cm:DesktopIconTask}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\target\release\walgit.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\target\release\walgit-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
; 已有配置绝不覆盖;卸载也不删(用户数据)
Source: "walgit.toml.initial"; DestDir: "{app}"; DestName: "walgit.toml"; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
Name: "{group}\walgit 托盘"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\walgit 配置文件 walgit.toml"; Filename: "notepad.exe"; Parameters: """{app}\walgit.toml"""
Name: "{autodesktop}\walgit 托盘"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "walgit-tray"; ValueData: """{app}\walgit-tray.exe"""; Flags: uninsdeletevalue; Tasks: autostart

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchTray}"; Flags: nowait postinstall skipifsilent

[Code]
procedure KillRunning(const exe: String);
var
  ResultCode: Integer;
begin
  // 正在运行的托盘/服务会锁住要替换的 exe;静默结束,失败忽略(多半本就没在跑)
  Exec(ExpandConstant('{cmd}'), '/C taskkill /IM "' + exe + '" /T /F', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
  begin
    KillRunning('walgit-tray.exe');
    KillRunning('walgit.exe');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    KillRunning('walgit-tray.exe');
    KillRunning('walgit.exe');
  end;
end;
