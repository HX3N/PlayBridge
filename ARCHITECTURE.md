# PlayBridge Architecture

PlayBridge lets [MAA (MaaAssistantArknights)](https://github.com/MaaAssistantArknights/MaaAssistantArknights)
drive Arknights running inside **Google Play Games on PC (GPG)** — a target MAA has no native support for.
It does this by impersonating the two interfaces MAA already speaks: an **ADB server** and a MuMu emulator's
**`external_renderer_ipc.dll`**. GPG itself is never patched; PlayBridge captures its window with Windows
Graphics Capture (WGC) and posts Win32 input to it.

## Two artifacts, one crate

A single `cargo build` produces both halves (see `Cargo.toml`):

| Artifact                    | Crate type | Source        | Pretends to be           | Used for                                                                  |
| --------------------------- | ---------- | ------------- | ------------------------ | ------------------------------------------------------------------------- |
| `PlayBridgeADB.exe`         | `bin`      | `src/main.rs` | the `adb` executable     | control, input, RawByNc screencap, hosting the WGC daemon                 |
| `external_renderer_ipc.dll` | `cdylib`   | `src/lib.rs`  | MuMu's nemu renderer DLL | the PlayExtras (MuMuExtras) fast screencap path, loaded in-process by MAA |

The DLL ships renamed to `<fakemumu>/nx_device/15.0/shell/sdk/external_renderer_ipc.dll`; MAA's "MuMu emulator
path" points at `<fakemumu>`. Release builds use `panic = "abort"`, so a panic inside the DLL takes MAA down —
every nemu export must be panic-free and turn errors into non-zero return codes.

## Component map

```
MAA (MaaAssistantArknights)
   │
   ├─ control / input ───────────►  PlayBridgeADB.exe        (bin = fake adb, src/main.rs)
   │    adb shell ...                  │  parse_command → execute_command
   │                                   ├─ "--wgc-daemon" → wgc::run_daemon()      (spawns the daemon below)
   │                                   ├─ "-i"           → input::run_minitouch_daemon()  (resident, src/input.rs)
   │                                   └─ keyevent/text  → Win32 PostMessage ───────────────┐
   │                                                                                        │
   ├─ RawByNc screencap ─────────►  PlayBridgeADB.exe (Command::ScreencapNc)                │
   │    exec-out screencap |           │  deliver_via_daemon(maa_port)                      │
   │    nc -w 3 10.0.2.2 <port>        ▼                                                    │
   │                              ┌──────────────────────────────────┐                      │
   └─ PlayExtras screencap ─────► │  WGC daemon          (src/wgc.rs) │                     │
        loads external_renderer_  │  `--wgc-daemon`, long-lived       │                     │
        ipc.dll (cdylib,          │  warm WgcCapture on GPG top-level │                     │
        src/lib.rs) →             │  caches latest BGRA frame         │                     │
        nemu_capture_display →    │  crop → resize 1280x720 → RGBA    │                     │
        TCP 127.0.0.1:<dport>     └──────────────┬───────────────────┘                      │
                                                 │ Windows Graphics Capture                 │
                                                 ▼                                          ▼
                                        GPG top-level window                           Win32 messages
                                          └ CROSVM_1 child (Arknights render surface)  ◄───┘
```

Both screencap paths converge on the **single** WGC daemon: PlayExtras gets the frame returned over the same
socket (byte-return verb), RawByNc has the daemon connect out to MAA's `nc` port. Input never goes through the
DLL — the nemu input exports are stubs; real input is Win32 `PostMessage` to the CROSVM child, from the bin or
the minitouch daemon.
The resident minitouch daemon posts a `MinitouchStopped` toast when its stdin closes.

Beyond connect/screencap, the bin handles a few one-shot ADB commands (`execute_command`):

- `am start -n <intent>` → resolves the client from the intent package (`apply_intent_package`) and launches the
  game if needed; a package that contradicts the configured client raises a `ClientMismatch` /
  `UnsupportedClient` toast instead
- `am force-stop` / `input keyevent HOME` → `WM_CLOSE` to the CROSVM child (`ForceStop`) + shutdown toast
- `shell echo <text>` → echoes the text back (MAA's "Compatible Mode" connection preset)
- `input tap` / `input swipe` → `AdbInputUnsupported` toast — raw ADB input is dropped, minitouch only
- `input keyevent` (ESC) / `input text` → Win32 `PostMessage` to the CROSVM child
- `--touch-overlay` → toggles touch-path overlay capture (`TOUCH_OVERLAY`, see _Shared state_)
- no arguments → saves a desktop PNG screenshot (`capture::screenshot`, the one remaining PrintWindow path) and
  runs the update check

## Connect handshake (PlayExtras + Minitouch)

MAA's `MuMuEmulator12` connect path issues a fixed sequence of ADB commands. `PlayBridgeADB.exe` answers each
with whatever keeps MAA moving forward. Call stack on MAA's side:
`Controller::connect()` → `MinitouchController::connect()` → `AdbController::connect()` → `probe_minitouch()`.

| #   | MAA ADB command                                                                | `main.rs` `Command`       | PlayBridge response / effect                                                  |
| --- | ------------------------------------------------------------------------------ | ------------------------- | ----------------------------------------------------------------------------- |
| 1   | `adb devices`                                                                  | `Devices`                 | prints `127.0.0.1:6000\tdevice` and a `PlayBridge <version>` line; runs update check                 |
| 2   | `adb connect <addr>`                                                           | `Connect`                 | `connected to Google Play Games`                                              |
| 3   | `settings get secure android_id`                                               | `GetUuid`                 | `0000000000000000`                                                            |
| 4   | `getprop ro.build.version.release`                                             | `GetPropRelease`          | `14` (faked Android version)                                                  |
| 5   | `wm size`                                                                      | `WindowDisplays`          | `1280 720`; **arms the one-shot benchmark flag**                              |
| 6   | `cat /proc/net/arp \| grep :`                                                  | `Ignore`                  | silent (no arp table to fake)                                                 |
| —   | _(socket server init, nemu DLL `nemu_connect` + first `nemu_capture_display`)_ | —                         | DLL spawns/warms the WGC daemon; not an ADB call                              |
| 7   | `getprop ro.product.cpu.abilist`                                               | `GetPropAbilist`          | abilist string (selects the minitouch binary)                                 |
| 8   | `dumpsys input \| grep SurfaceOrientation`                                     | `DumpsysInputOrientation` | `0`                                                                           |
| 9   | `push <minitouch> /data/local/tmp/<uuid>`                                      | `Ignore`                  | silent (no upload needed)                                                     |
| 10  | `chmod 700 /data/local/tmp/<uuid>`                                             | `Ignore`                  | silent                                                                        |
| 11  | `shell /data/local/tmp/<uuid> -i`                                              | _(main.rs `-i` branch)_   | `input::run_minitouch_daemon()` — resident, reads minitouch commands on stdin |

After connect returns, MAA benchmarks its screencap modes (RawByNc, RawWithGzip, Encode, PlayExtras DLL) and
locks onto the fastest. See _Benchmark one-shot_ for how PlayBridge keeps that measurement fair.

## Supported screencap modes

Of MAA's screencap modes, this project supports **two**, both served by the WGC daemon; the rest are silently
ignored so MAA falls back instead of showing an "unknown command" toast.

| Mode                        | Trigger                                         | Path                                                                                                                    |
| --------------------------- | ----------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------- |
| **PlayExtras** (MuMuExtras) | DLL `nemu_capture_display`                      | `lib.rs` → TCP `127.0.0.1:<dport>` with port==0 → daemon writes `[w: u32][h: u32][rgba]` back on the same socket          |
| **RawByNc**                 | `exec-out screencap \| nc -w 3 10.0.2.2 <port>` | `main.rs ScreencapNc` → `deliver_via_daemon(port)` → daemon connects out to MAA's `nc` port and streams RGBA, then ACKs |
| RawWithGzip                 | `exec-out screencap \| gzip -1`                 | `Ignore` (unsupported, silent)                                                                                          |
| Encode                      | `exec-out screencap -p`                         | `Ignore` (unsupported, silent)                                                                                          |

MAA applies `cvtColor(RGBA2BGR)` then `flip(.,0)` to PlayExtras frames, so the DLL writes the frame **bottom-up**
to cancel that vertical flip (`nemu_capture_display`). RawByNc frames are sent top-down.

With no fresh frame to serve (nothing captured yet, or the cached frame went stale), both verbs degrade to a
black frame in the same format. If the daemon isn't running at all, `ScreencapNc` spawns it and answers that one
request with a black frame itself; the next request hits the warm daemon.

## Benchmark one-shot

`wm size` (#5; `dumpsys window displays` maps to the same handler) sets a one-shot `BENCHMARK` flag in the
registry. While it is armed:

- `main()` skips `ensure_game_ready()` (`peek_benchmark_mode`), so a ~1s game launch can't skew the timing.
- the next RawByNc request returns a **black frame** immediately (`check_benchmark_mode` consumes the flag),
  so a cold daemon spawn doesn't penalize RawByNc's measured throughput.

The flag is consumed on that single capture; normal capture resumes on the next request.

## Shared state (registry)

The bin and the DLL are separate processes; all persistent state lives under `HKCU\Software\PlayBridge`. The
`…\state` keys are the bin↔DLL rendezvous; `…\config` and `…\cooldown` are bin-local:

| Subkey (`config.rs`) | Value                | Written by              | Read by                  | Purpose                                                    |
| -------------------- | -------------------- | ----------------------- | ------------------------ | ---------------------------------------------------------- |
| `…\state`            | `WGC_DAEMON_PORT`    | daemon (`run_daemon`)   | bin, DLL                 | daemon's TCP port, `0` while down                          |
| `…\state`            | `EXE_PATH`           | `main()`                | DLL                      | lets the DLL spawn `--wgc-daemon` without knowing its path |
| `…\state`            | `BENCHMARK`          | `wm size` handler       | `ScreencapNc` / `main()` | one-shot benchmark gate                                    |
| `…\config`           | `CLIENT`, `VERSION`  | bin                     | bin                      | client (EN/KR/JP) + last-seen version                      |
| `…\config`           | `TOUCH_OVERLAY`      | bin (`--touch-overlay`) | bin                      | touch-path overlay capture toggle                          |
| `…\config`           | `LAST_UPDATE_CHECK`  | bin                     | bin                      | GitHub release check throttle (24 h)                       |
| `…\cooldown`         | `<notification tag>` | bin                     | bin                      | per-toast throttle timestamps (`display_notification`)     |

`src/lib.rs` hardcodes `REG_STATE`, `KEY_DAEMON_PORT`, and `KEY_EXE_PATH` rather than importing them (the cdylib
doesn't share the bin's modules) — **these must stay in sync with `config.rs` and `wgc.rs`.**

## WGC daemon lifecycle

- **Single instance**, guarded by a named mutex (`daemon_already_running`). Binds `127.0.0.1:0`, publishes the
  port, then serves frames from a warm `WgcCapture` session on the GPG top-level window.
- A dedicated accept thread feeds a channel for zero accept latency; the main loop also runs a **maintenance
  tick** every 200 ms (`maintain`) that restores/rebinds the window — fresh frames, not `IsWindow`, are the
  liveness signal (a cached frame older than 1 s counts as stale and is served as black).
- **Rebinds build off the serve thread**: constructing a `WgcCapture` is the one slow step, so `spawn_build` runs
  it on a worker (`building` guards against duplicates) and the old capture keeps serving until the new one lands.
- While no window is bound (startup, or waiting out a game restart), the daemon **relaunches the game itself**
  via `ensure_game_ready`, throttled to one attempt per 10 s.
- On every (re)bind, `check_render_resolution` reads the per-package render resolution GPG keeps in `store.db`
  (`src/store.rs`): a non-16:9 value raises a `WindowWrongRatio` toast, a 16:9 value other than 1280x720 raises
  `InternalResolution` — once per value.
- **Self-exits** on idle timeout (120 s) or when the window is truly gone, so a fresh daemon rebinds after the
  game relaunches. On exit it restores the window's default rounded corners, clears the port, and posts a
  `WgcDaemonStopped` toast.

See `src/wgc.rs` for the capture/crop/resize details (notably `crop_region`, which pads the right/bottom edge
DWM leaves transparent).
