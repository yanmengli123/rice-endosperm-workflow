# Pet

Wisp Science supports one user-selected Codex-compatible v2 animated pet. Pet support is off by default and the app does not scan a default directory.

## Enable a pet

1. Open **Settings > Pet**.
2. Choose the pet installation folder. The folder must contain `pet.json` and the spritesheet named by `spritesheetPath`.
3. Turn on **Show pet** and save.

The pet folder must use `spriteVersionNumber: 2` and contain a `1536x2288` PNG or WebP atlas arranged as 8 columns by 11 rows of `192x208` cells. Replacing the configured folder, or replacing its compatible files and saving the setting again, changes the active pet. Turning **Show pet** off stops loading and displaying it.

The app uses the standard animation rows for idle, directional walking, waving, jumping, failure, waiting for user input, active work, review, and 16-direction pointer tracking. Clicking an idle pet makes it wave. Reduced-motion system preferences disable roaming and animated playback.

On Windows, the pet lives in its own transparent always-on-top window instead of inside the main workspace. Drag the pet to move it. While a managed Run is active, the pet shows up to two lines of its title above the sprite; when it finishes, the pet celebrates. When it shows **Needs you**, click it to restore the workspace and open the session waiting for input. Closing the main Wisp Science window hides the workspace to the system tray while the pet and active agent work continue; click the tray icon, choose **Open Wisp Science**, or launch Wisp Science again to restore the existing workspace. Only one desktop app instance runs at a time. Choose **Restart** to restart it or **Quit** to stop it. The pet animation and status badge show when the agent is working, reviewing, waiting for approval, finished, or failed.

## 配置宠物

Wisp Science 只加载一个由用户明确选择的 Codex v2 宠物，默认关闭，也不会自动扫描任何目录。

打开 **设置 > 宠物**，选择包含 `pet.json` 和精灵表的安装目录，开启 **显示宠物** 后保存。目录必须使用 `spriteVersionNumber: 2`，精灵表必须是 `1536x2288` 的 PNG 或 WebP 文件。更换配置目录即可更换宠物；关闭开关后，应用不会再加载或显示该目录中的宠物。

在 Windows 上，宠物运行在独立的透明置顶窗口中，而不是嵌在主工作区内，可以拖动到屏幕上的其他位置。托管 Run 执行时会在宠物上方显示最多两行任务名称，完成时宠物会跳动提醒。显示 **Needs you** 时，点击宠物会恢复工作区并打开正在等待操作的 session。关闭 Wisp Science 主窗口会把工作区隐藏到系统托盘，宠物和正在执行的 agent 任务会继续工作；点击托盘图标、选择 **打开 Wisp Science** 或再次启动 Wisp Science 都会恢复现有工作区。桌面应用全局只运行一个实例；选择 **重启** 会重启应用，选择 **退出** 才会真正退出。宠物动画、状态点和文字会显示工作中、review、等待批准、完成或失败状态。
