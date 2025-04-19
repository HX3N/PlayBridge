> [!Note]
> This repository is a **customized fork** of [PlayBridge](https://github.com/ACK72/PlayBridge).

### Setup

Download [PlayBridgeADB.exe](https://github.com/HX3N/PlayBridge/releases/latest) and place it inside the MAA folder.<br>
Then, go to **MAA > Settings > Connection Settings** and configure as follows:

| ADB Path      | Connection Address | Input Method |
| ------------- | ------------------ | ------------ |
| PlayBridgeADB | 127.0.0.1:5000     | ADB Input    |

### Config

To enable custom settings, create a file named `PlayBridgeADB.json` in the same directory as the exe file.

<details>
  <summary><strong>Default <code>PlayBridgeADB.json</code></strong></summary>

```json
{
  "title": "명일방주",
  "package": "com.YoStarKR.Arknights",
  "swipe_speed": 10,
  "width": 1280,
  "height": 720,
  "min_width": 1024,
  "min_height": 576,
  "debug": false,
  "notification": true
}
```

</details>

<details>
  <summary><strong>Descriptions of each key</strong></summary>

| Key            | Description                                                    |
| -------------- | -------------------------------------------------------------- |
| `title`        | Window title pattern used to locate the game window            |
| `package`      | Arknights package name used to launch the game                 |
| `swipe_speed`  | Multiplier for swipe speed                                     |
| `width`        | Resolution used when resizing images for MAA or for screenshot |
| `height`       | **Should maintain a 16:9 aspect ratio**                        |
| `min_width`    | Minimum window resolution required for proper MAA recognition  |
| `min_height`   | **Should maintain a 16:9 aspect ratio**                        |
| `debug`        | Enable or disable debug logging                                |
| `notification` | Enable or disable system notifications                         |

</details>

### Screenshot

Running the executable directly captures a screenshot of **Google Play Games Beta** and saves it to the Desktop.<br>
The image resolution follows the `width` / `height` settings.

### Notification

Displays Windows toast notifications for events.<br>
Used to inform the user when something goes wrong or when key actions succeed or fail.
