> [!Note]
> This repository is a **customized fork** of [PlayBridge](https://github.com/ACK72/PlayBridge)  
> For general use, the original one is recommended

### Setup

> The **Auto Reload** feature in MAA Debug mode will only work if the connection address is set as shown below

Download [PlayBridgeADB](https://github.com/HX3N/PlayBridge/releases/latest) and place it inside the MAA folder<br>
Then, go to **MAA > Settings > Connection** and configure as follows:

<div align="center">

![img](./assets/readme.png)

| ADB path      | Connection address | Connection Preset       | Touch Mode |
| ------------- | ------------------ | ----------------------- | ---------- |
| PlayBridgeADB | GooglePlayGames    | General/Compatible Mode | ADB Input  |

</div>

### Config

To enable custom settings, run `PlayBridgeConfig.bat` and change the values<br>
Or, you can change the registry values directly in `Software\PlayBridge\config`

<div align="center">

| Variable        | Description                     | Default                |
| --------------- | :------------------------------ | ---------------------- |
| `TITLE`         | Window title (server)           | 명일방주               |
| `PACKAGE`       | Arknights package name (server) | com.YoStarKR.Arknights |
| `SWIPE_SPEED`   | Swipe speed multiplier          | 10                     |
| `MAX_FPS`       | Maximum FPS for Extras capture  | 10                     |
| `DEBUG`         | debug logging                   | false                  |
| `DEBUG_CAPTURE` | debug capture                   | false                  |

</div>

### Screenshot

Running the executable directly captures a screenshot of **Google Play Games** and saves it to the Desktop

### Extras (Test)

> To activate it, place the `PlayBridgeExtras` in the same directory as the `PlayBridgeADB`

PlayBridgeExtras can improve the screenshot time, but it consumes more CPU<br>
If Google Play Games is closed or the 45 seconds have passed, Extras will automatically exit
