> [!NOTE]
> For general use cases, it is recommended to use the **original [PlayBridge](https://github.com/ACK72/PlayBridge)**<br>This repository is a customized fork

> [!IMPORTANT]
>
> ### To check for any issues, use the **`Peep`** feature in MAA's Toolbox tab
>
> The Google Play Games screen must be in `16:9 aspect ratio`

## Setup

Download [PlayBridgeADB](https://github.com/HX3N/PlayBridge/releases/latest) and place it inside the MAA folder<br>
Then, go to **MAA > Settings > Connection** and configure as follows:

<div align="center">

![img](./assets/readme.png)

| ADB path      | Connection address | Connection Preset       | Touch Mode |
| ------------- | ------------------ | ----------------------- | ---------- |
| PlayBridgeADB | GooglePlayGames    | General/Compatible Mode | ADB Input  |

</div>

## Debug Capture

Run `.\PlayBridgeADB --debug` in the directory where the executable is located to toggle debug mode<br>
You can also modify the registry value directly at `Software\PlayBridge\config\DEBUG_CAPTURE`

## Screenshot

Running the executable directly captures a screenshot of **Google Play Games** and saves it to the Desktop
