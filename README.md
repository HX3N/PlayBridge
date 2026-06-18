<div align="center">

**English** | [한국어](README.ko.md)

</div>

> [!IMPORTANT]
> This is a customized fork of the original [PlayBridge](https://github.com/ACK72/PlayBridge).

## Setup

> [!TIP]
> To check for any issues, use the **`Peep`** feature in MAA's Toolbox tab.<br>
> Recommended to enable `Login` feature on the first launch.

**PlayExtras** is the default capture path — a faster screencap built on **Windows Graphics Capture (WGC)**, averaging **5–20 ms**.

1. Download **[`Install.bat`](https://github.com/HX3N/PlayBridge/releases/latest)** and run it from your MAA folder. It downloads the latest `PlayBridgeADB.exe` and `external_renderer_ipc.dll` and places them automatically (creating `PlayExtras\nx_device\15.0\shell\sdk\` for the DLL).
2. Then, go to **MAA > Settings > Connection** and configure as follows:

<div align="center">

![img](./assets/PlayExtras2.png)

| ADB path          | Connection address | Connection Preset | Touch Mode | MuMu Installation Path | Screenshot enhancement mode |
| ----------------- | ------------------ | ----------------- | ---------- | ---------------------- | --------------------------- |
| PlayBridgeADB.exe | GooglePlayGames    | MuMu Emulator     | Minitouch  | `PlayExtras`           | Enabled                     |

</div>

<details>
<summary>How it works</summary>

PlayExtras impersonates MAA's **MumuExtras** flow: MAA loads a vendor `external_renderer_ipc.dll` in-process and calls the nemu ABI to grab the framebuffer. PlayBridge ships a drop-in DLL that exports the same ABI but pulls frames from the WGC daemon instead. The daemon starts on demand, self-exits when idle, and falls back to the standard capture path if it can't run.

</details>

## Screenshot

Running the executable directly captures a screenshot of **Google Play Games** and saves it to the Desktop

## Debug Capture

Run `.\PlayBridgeADB --debug` in the directory where the executable is located to toggle debug mode<br>
You can also modify the registry value directly at `Software\PlayBridge\config\DEBUG_CAPTURE`

## RawByNc (legacy)

> [!WARNING]
> Not recommended — slower than PlayExtras. Use it only if PlayExtras can't run.

Download [PlayBridgeADB](https://github.com/HX3N/PlayBridge/releases/latest) and place it inside the MAA folder, then configure:

<div align="center">

![img](./assets/RawByNc2.png)

| ADB path      | Connection address | Connection Preset | Touch Mode |
| ------------- | ------------------ | ----------------- | ---------- |
| PlayBridgeADB | GooglePlayGames    | General Mode      | Minitouch  |

</div>
