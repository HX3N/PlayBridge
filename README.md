> [!Note]
> This repository is a **customized fork** of [PlayBridge](https://github.com/ACK72/PlayBridge).

### Setup

Download [PlayBridge-adb.exe](https://github.com/HX3N/PlayBridge/releases/latest) and place it inside the MAA folder.<br>
Then, go to **MAA > Settings > Connection Settings** and configure as follows:

| ADB Path           | Connection Address | Input Method |
| ------------------ | ------------------ | ------------ |
| PlayBridge-adb.exe | 127.0.0.1:5000     | ADB Input    |

### Config

You can enter config mode by including `-config` in the executable name and running it.

Available settings:

- `REGION`: Select the server region **KR / EN / JP**
- `SWIPE_SPEED`: Adjust the swipe speed (Recommended 4~16)
- `DISPLAY_WIDTH` / `DISPLAY_HEIGHT`: Set the resolution of the image passed to MAA

### Screenshot

Running the executable directly captures a screenshot of **Google Play Games Beta** and saves it to the Desktop.

> The image resolution follows the `DISPLAY_WIDTH` / `DISPLAY_HEIGHT` settings.

### Notification

Displays Windows toast notifications for events such as:

- Resolution changes
- Unknown or unsupported commands
- Low resolution or incorrect aspect ratio (non-16:9)
