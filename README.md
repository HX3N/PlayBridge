> [!Note]
> This repository is a **customized fork** of [PlayBridge](https://github.com/ACK72/PlayBridge)  
> For general use, the original one is recommended

### Setup

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

| Variable        | Description            | Default                |
| --------------- | :--------------------- | ---------------------- |
| `TITLE`         | Window title           | 명일방주               |
| `PACKAGE`       | Arknights package name | com.YoStarKR.Arknights |
| `SWIPE_SPEED`   | Swipe speed multiplier | 10                     |
| `DEBUG`         | debug logging          | false                  |
| `DEBUG_CAPTURE` | debug capture          | false                  |

</div>

### Screenshot

Running the executable directly captures a screenshot of **Google Play Games** and saves it to the Desktop
