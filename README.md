> [!Note]
> This repository is a **customized fork** of [PlayBridge](https://github.com/ACK72/PlayBridge)

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

To enable custom settings, create a `PlayBridge` folder in the same directory as the exe file and create a `config.json` file

<details>
  <summary><strong>config.json</strong></summary>

```json
{
  // Default values
  "title": "명일방주",
  "package": "com.YoStarKR.Arknights",
  "swipe_speed": 10,
  "debug": false,
  "debug_capture": false
}
```

<div align="center">

| Key             | Description                     |
| --------------- | :------------------------------ |
| `title`         | Window title (server)           |
| `package`       | Arknights package name (server) |
| `swipe_speed`   | Swipe speed multiplier          |
| `debug`         | debug logging                   |
| `debug_capture` | debug capture                   |

</div>
</details>

### Screenshot

Running the executable directly captures a screenshot of **Google Play Games** and saves it to the Desktop

### Extras (Test)

> To activate it, place the `PlayBridgeExtras` in the same directory as the `PlayBridgeADB`

PlayBridgeExtras can improve the screenshot time, but it consumes more CPU<br>
If Google Play Games is closed or the 45 seconds have passed, Extras will automatically exit
