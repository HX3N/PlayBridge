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

## Module layout

`src/` is grouped by which process the code runs in, so a path tells you the runtime:

| Path                          | Runs in                        | Holds                                                                     |
| ----------------------------- | ------------------------------ | ------------------------------------------------------------------------- |
| `src/main.rs`                 | every bin invocation           | DPI/logging setup, argv parsing, dispatch to a daemon or the shim         |
| `src/shim.rs`                 | the one-shot fake-adb call     | `Command`, `parse_command`, `execute_command`                             |
| `src/daemon/`                 | the long-lived processes       | `wgc`, `minitouch`, `launcher`, and the `window_state` the WGC daemon owns |
| `src/game/`                   | anything touching GPG's window | `window` (finding it, and the input gate), `input`, `capture`             |
| `src/sys/`                    | anything                       | registry config, logging, toasts, MAA lookup, GPG store, named mutexes    |
| `src/lib.rs`, `src/shared.rs` | the cdylib (and the bin)       | nemu exports; the constants both crates must agree on                     |

Dependencies only point down: `daemon/` may use `game/` and `sys/`, `game/` may use `sys/`, and `sys/` uses
nothing above it. `shared.rs` stays at the root because the bin and the cdylib are separate crates that each
declare `mod shared;` over that one file.

## Component map

```
MAA (MaaAssistantArknights)
   │
   ├─ control / input ───────────►  PlayBridgeADB.exe        (bin = fake adb, src/main.rs)
   │    adb shell ...                  │  parse_command → execute_command  (src/shim.rs)
   │                                   ├─ "--wgc-daemon"      → wgc::run_daemon()   (spawns the daemon below)
   │                                   ├─ "--launcher-daemon" → launcher::run_launcher_daemon()
   │                                   │                        (short-lived, starts GPG then exits)
   │                                   ├─ "-i"                → minitouch::run_minitouch_daemon()  (resident, src/daemon/minitouch.rs)
   │                                   └─ keyevent/text       → Win32 PostMessage ────────────┐
   │                                                                                        │
   ├─ RawByNc screencap ─────────►  PlayBridgeADB.exe (Command::ScreencapNc)                │
   │    exec-out screencap |           │  deliver_via_daemon(maa_port)                      │
   │    nc -w 3 10.0.2.2 <port>        ▼                                                    │
   │                              ┌──────────────────────────────────┐                      │
   └─ PlayExtras screencap ─────► │  WGC daemon   (src/daemon/wgc.rs) │                     │
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

The daemon also owns the GPG window's state, not just its pixels: it normalizes the window every maintenance
tick, intercepts minimize so capture never stops (see _Window parking_), and clears an activation the window
failed to release (see _Stale activation_). Both live in `src/daemon/window_state.rs`; `src/daemon/wgc.rs` keeps the capture
pipeline. Starting the game is not its job — that belongs to the launcher daemon (see _Client selection and
launch_).

Beyond connect/screencap, the bin handles a few one-shot ADB commands (`execute_command`):

- `am start -n <intent>` → checks the intent package against the client MAA already chose
  (`apply_intent_package`) and spawns the launcher daemon; a package that contradicts it raises a
  `ClientMismatch` / `UnsupportedClient` toast instead
- `am force-stop` / `input keyevent HOME` → `WM_CLOSE` to the CROSVM child (`ForceStop`) + shutdown toast
- `shell echo <text>` → echoes the text back (MAA's "Compatible Mode" connection preset)
- `dumpsys SurfaceFlinger --latency` → prints `16666666`, the 60fps frame period in ns MAA's fps probe reads
- `input tap` / `input swipe` → `AdbInputUnsupported` toast — raw ADB input is dropped, minitouch only
- `input keyevent` (ESC) / `input text` → Win32 `PostMessage` to the CROSVM child
- `--touch-overlay` → toggles touch-path overlay capture (`TOUCH_OVERLAY`, see _Shared state_)
- no arguments → saves a desktop PNG screenshot (`capture::screenshot`) and runs the update check

The two PNG-writing paths (`capture::screenshot` and `capture_touch_overlay`) are the only remaining users of
the synchronous `PrintWindow` readback; every frame MAA actually consumes comes from WGC.

The update check (`check_for_update`) compares the built-in version against the latest GitHub release and
raises an `UpdateAvailable` toast when they differ. The toast carries a protocol-activated button to the
releases page: Windows performs the activation, so the button still works after this short-lived process has
exited.

## Connect handshake (PlayExtras + Minitouch)

MAA's `MuMuEmulator12` connect path issues a fixed sequence of ADB commands. `PlayBridgeADB.exe` answers each
with whatever keeps MAA moving forward. Call stack on MAA's side:
`Controller::connect()` → `MinitouchController::connect()` → `AdbController::connect()` → `probe_minitouch()`.

| #   | MAA ADB command                                                                | `shim.rs` `Command`       | PlayBridge response / effect                                                  |
| --- | ------------------------------------------------------------------------------ | ------------------------- | ----------------------------------------------------------------------------- |
| 1   | `adb devices`                                                                  | `Devices`                 | prints `127.0.0.1:6000\tdevice` and a `PlayBridge <version>` line; **resolves the client from MAA's own config and preloads the launcher** |
| 2   | `adb connect <addr>`                                                           | `Connect`                 | `connected to Google Play Games`                                              |
| 3   | `settings get secure android_id`                                               | `GetUuid`                 | `0000000000000000`                                                            |
| 4   | `getprop ro.build.version.release`                                             | `GetPropRelease`          | `14` (faked Android version)                                                  |
| 5   | `wm size`                                                                      | `WindowDisplays`          | `1280 720`                                                                    |
| 6   | `cat /proc/net/arp \| grep :`                                                  | `Ignore`                  | silent (no arp table to fake)                                                 |
| —   | _(socket server init, nemu DLL `nemu_connect` + first `nemu_capture_display`)_ | —                         | DLL spawns/warms the WGC daemon; not an ADB call                              |
| 7   | `getprop ro.product.cpu.abilist`                                               | `GetPropAbilist`          | abilist string (selects the minitouch binary)                                 |
| 8   | `dumpsys input \| grep SurfaceOrientation`                                     | `DumpsysInputOrientation` | `0`                                                                           |
| 9   | `push <minitouch> /data/local/tmp/<uuid>`                                      | `Ignore`                  | silent (no upload needed)                                                     |
| 10  | `chmod 700 /data/local/tmp/<uuid>`                                             | `Ignore`                  | silent                                                                        |
| 11  | `shell /data/local/tmp/<uuid> -i`                                              | _(main.rs `-i` branch)_   | `minitouch::run_minitouch_daemon()` — resident, reads minitouch commands on stdin |

MAA starts minitouch (#11) as soon as connect returns, which can be before the game window exists. The
handshake has already told MAA the daemon is up by then, so it never exits on a missing window — it binds on
the first commit that finds one and skips commits until then.

After connect returns, MAA benchmarks its screencap modes (RawByNc, RawWithGzip, Encode, PlayExtras DLL) and
locks onto the fastest.

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

The DLL sits on MAA's hot path, so it avoids per-frame cost: the daemon port is cached in a static after the
first successful connect (a failed connect clears it, so a daemon that restarted on a new port is picked up),
and the ~3.7 MB frame arrives into one reused staging buffer instead of a fresh allocation per capture.

Frame freshness is decided per request (`handle_client`). Arknights animates continuously, so a frame older
than **1 s** (`FRAME_STALE`) means the window stopped composing — but stopped composing is not dead: a static
overlay (the GPG user center webview) stalls WGC while the last frame is still the current screen. So a stale
frame is **still served, up to 30 s** (`FRAME_REUSE_LIMIT`), as long as the bound child window is alive; the
delivery is logged at `Warn` as a reuse. Past that bound, or with nothing captured at all, both verbs degrade
to a black frame in the same format. If the daemon isn't running at all, `ScreencapNc` spawns it and answers
that one request with a black frame itself; the next request hits the warm daemon.

## Finding the game window

Everything that captures or types — the WGC daemon, the minitouch daemon, the one-shot commands — reaches the
game through `GameWindow::find()` (`src/game/window.rs`). GPG nests the render surface two levels down:

```
HwndWrapper[DefaultDomain;;<guid>]   top-level, owned by crosvm.exe, titled "<game name> - <doctor>"
  └ CROSVM_1                         what PlayBridge binds to (capture, PostMessage input)
      └ subWin
```

The search makes **one** pass over `window_list()` and puts each window through two gates: its title must start
with one of the three client names (`명일방주` / `アークナイツ` / `Arknights`), and it must own a direct `CROSVM_1`
child. The first window clearing both yields the child handle *and* the client the title identified — finding
the window and identifying the region are the same step, which is why nothing has to be searched per client.

Both gates carry weight. The title prefix is what keeps a GPG session running some *other* game from being
handed to MAA, and it absorbs the `<game name> - <doctor>` suffix. The `CROSVM_1` check is what rejects GPG's
own chrome. And the pass leans on `window_list()` yielding only visible, titled windows: GPG keeps four more
top-levels of the same class, each with a `CROSVM_1` child of its own, but all four are invisible 12x12 stubs
with empty titles and never reach the filter.

## Client selection and launch

`adb devices` is where the client (EN/KR/JP) is first **proposed**, not where it is settled. Only that process
has MAA as its parent, so `src/sys/maa.rs` walks up to it (`CreateToolhelp32Snapshot` → `th32ParentProcessID`),
reads `<MAA>/config/gui.new.json` and takes `Configurations[<Current>].Gui.RuntimeSettings.ClientType`. MAA's
`Official`, `Bilibili` and `Txwy` have no Google Play Games package, so they raise an `UnsupportedClient` toast
and nothing is launched — and nothing is stored either, so `CLIENT` keeps whatever it already held. An
unreadable config leaves the client to `GameWindow::find()` (see _Finding the game window_).

The same handler stores the resolved client in `CLIENT` and MAA's PID in `MAA_PID`. That is how the daemons
learn both — they are spawned `DETACHED_PROCESS`, so they cannot walk up to MAA themselves.

`devices` arrives once per MAA process, so a user whose MAA points at the wrong client would otherwise be stuck
with it for the whole session. Two later points correct `CLIENT` against what is actually on the machine. Their
preconditions are exclusive: a running window rules the store lookup out, because the launcher exits at
"game already up" before reaching it.

| Evidence                     | Where                                             | Precondition |
| ---------------------------- | ------------------------------------------------- | ------------ |
| the window that is really up | `GameWindow::find` → `adopt_running_client`       | GPG running  |
| GPG's own install record     | `run_launcher_daemon` → `adopt_installed_client`  | GPG closed   |

The window search already reports which client's title it matched, so `adopt_running_client` takes that as the
truth and rewrites `CLIENT`; it returns early when the two agree, which is the ordinary case. The launcher's
lookup instead asks `store::app_record` for each of the other two clients and claims one only when exactly one
answers; two installed side by side is assumed not to happen and is not guessed at. Both write `CLIENT` before
raising their `ClientMismatch` toast, because `sys/notification.rs` picks the toast's language from that same value.

A corrected `CLIENT` is also what makes `apply_intent_package` speak up: MAA keeps sending the intent for the
client it still believes in, which now contradicts the stored one and raises `ClientMismatch` on its own.

Launching is a **short-lived process of its own** (`--launcher-daemon`, `daemon/launcher.rs`), spawned from `devices` as
a preload because GPG takes seconds to come up. Its lifetime *is* the "game is starting" signal, so nothing
else tracks launch state — capture and input simply find no window until it exits.

- Single instance (`Local\PlayBridgeLauncher`). `ensure_launcher` tests the mutex with `OpenMutexW` instead of
  creating it: creating it would make the caller the owner, and every later launcher would see a daemon that
  never started.
- Exits at once if the game window is already up.
- Exits too when no client has been set, since `CLIENT` is what names the package to launch.
- Checks GPG's `store.db` for the package first (`store::app_record`). Without a record the launch URI only
  raises GPG's own window, which reads as a loading screen forever, so a miss falls through to
  `adopt_installed_client` and only raises a `GameNotInstalled` toast when that finds nothing to switch to.
  A hit is logged with the app version and both resolutions GPG keeps for it.
- Stops as soon as MAA does: every poll checks `maa::is_alive()` before anything else, so no URI fires once MAA
  is gone. It is spawned `DETACHED_PROCESS`, so nothing else would end it.
- Otherwise fires `googleplaygames://launch/?id=<package>&pid=1`, retrying every 10 s until the window appears
  and giving up after 180 s. A loading screen that vanishes without the game means the launch died partway, so
  the cooldown is cleared and the URI fires again.

The WGC daemon also calls `ensure_launcher` when MAA asks for a screen and there is none — that request is the
one moment worth starting the game. Nothing else launches it, which is why closing GPG after MAA has finished
no longer brings it back.

## Shared state (registry)

The bin and the DLL are separate processes; all persistent state lives under `HKCU\Software\PlayBridge`. The
`…\state` keys are the bin↔DLL rendezvous; `…\config` and `…\cooldown` are bin-local:

| Subkey (`sys/config.rs`) | Value                | Written by              | Read by                  | Purpose                                                    |
| -------------------- | -------------------- | ----------------------- | ------------------------ | ---------------------------------------------------------- |
| `…\state`            | `WGC_DAEMON_PORT`    | daemon (`run_daemon`)   | bin, DLL                 | daemon's TCP port, `0` while down                          |
| `…\state`            | `EXE_PATH`           | `main()`                | DLL                      | lets the DLL spawn `--wgc-daemon` without knowing its path |
| `…\state`            | `MAA_PID`            | `devices` handler       | WGC daemon               | the MAA that spawned the daemons; its exit ends them       |
| `…\state`            | `WINDOW_HOME`        | daemon (`ParkState`)    | daemon                   | `x,y` the window belongs at, survives daemon restarts      |
| `…\state`            | `PARK_HOME`          | daemon (`ParkState`)    | daemon                   | set only while parked; found at startup = died parked      |
| `…\config`           | `CLIENT`, `VERSION`  | bin                     | bin                      | client (EN/KR/JP) + last-seen version                      |
| `…\config`           | `TOUCH_OVERLAY`      | bin (`--touch-overlay`) | bin                      | touch-path overlay capture toggle                          |
| `…\config`           | `LAST_UPDATE_CHECK`  | bin                     | bin                      | GitHub release check throttle (24 h)                       |
| `…\cooldown`         | `<notification tag>` | bin                     | bin                      | per-toast throttle timestamps (`display_notification`)     |

The cdylib doesn't share the bin's modules, but both crate roots sit in `src/`, so each declares
`mod shared;` over `src/shared.rs` — the single definition of the `…\state` path, its key names, the
1280x720 display size, and the daemon's spawn flags. A value that drifts is a compile error, not a
silent mismatch.

## WGC daemon lifecycle

- **Single instance**, guarded by a named mutex (`already_running`). Binds `127.0.0.1:0`, publishes the
  port, then serves frames from a warm `WgcCapture` session on the GPG top-level window.
- A dedicated accept thread feeds a channel for zero accept latency; the main loop also runs a **maintenance
  tick** every 50 ms (`maintain`) that unparks/rebinds the window. Fresh frames, not `IsWindow`, are the
  liveness signal: while frames flow the bound child is reused, and once they stall past 1 s the window is
  re-verified through `GameWindow::find()` (see _Finding the game window_).
- **Rebinds build off the serve thread**: constructing a `WgcCapture` is the one slow step, so `spawn_build` runs
  it on a worker (`building` guards against duplicates) and the old capture keeps serving until the new one lands.
  A rebind is triggered by a changed child HWND *or* a changed client size — the frame pool is fixed to the
  window size, so a resize needs a new session even on the same HWND.
- While no window is bound (startup, or waiting out a game restart), the daemon **waits** — it does not launch
  anything. A capture request that finds no frame calls `ensure_launcher` on its way to returning black.
- On every (re)bind, `check_render_resolution` reads the per-package render resolution GPG keeps in `store.db`
  (`src/sys/store.rs`): a non-16:9 value raises a `WindowWrongRatio` toast, a 16:9 value other than 1280x720 raises
  `InternalResolution` — once per value.
- **Lives exactly as long as MAA.** It polls `MAA_PID` once a second and exits within ~1 s of MAA going away,
  the same rule the launcher polls for and the minitouch daemon gets for free from its stdin pipe. Idling does
  not end it: after 15 s with no request (`LOW_POWER_AFTER`) it drops to **low power**, discarding arriving
  frames instead of paying the GPU→CPU copy for nobody, and the next request restores full rate. On exit it
  restores the window's default rounded corners, un-parks the window if it is parked, and clears the published
  port.

See `src/daemon/wgc.rs` for the capture/crop/resize details (notably `crop_region`, which pads the right/bottom edge
DWM leaves transparent).

## Window parking (minimize mimicry)

Implemented in `src/daemon/window_state.rs`.

WGC stops delivering frames the moment a window is genuinely minimized, which would stall MAA for as long as
the user keeps GPG out of the way. Instead of rejecting the minimize, the daemon **fakes** it: the window stays
restored and composing, but is moved just below the virtual desktop (`park_y()` = bottom + 32 px). Only Y
moves, so the taskbar button stays on its own monitor and the window still looks minimized.

Two `SetWinEventHook` callbacks on a dedicated message-pump thread (`spawn_minimize_watcher`) plus the
maintenance-tick poll (`ParkState::update`) split the work:

| Trigger                    | Handler                                | Effect                                                              |
| -------------------------- | -------------------------------------- | ------------------------------------------------------------------- |
| `EVENT_SYSTEM_MINIMIZESTART` | `on_minimize_start`                  | moves the window off-screen **before** the minimize lands, so the move sticks and becomes the restore rect |
| `EVENT_SYSTEM_FOREGROUND`  | `on_foreground`                        | re-applies the stored return point when the user brings GPG back      |
| maintenance tick           | `ParkState::update` → `park`/`unpark`  | owns the state machine: parks on `IsIconic`, unparks when GPG is foreground |

Ordering is what makes it work. Once a window is iconic, `SetWindowPos` is silently ignored, so the hook has to
act during `MINIMIZESTART` while the window is still restored; the poll then finishes the job by
un-minimizing (`SW_SHOWNOACTIVATE`) at the already off-screen position. `SetWindowPlacement` is never used to
park, because it drags an off-screen `rcNormalPosition` back onto the primary monitor.

The window's real origin ("home") is learned from `GetWindowRect` while it is on a monitor and cached in
`WINDOW_HOME`. A minimized window reports a bogus origin, so if the daemon meets an already-minimized window
with no cached home it surfaces it for one pass (`pending_park`) to read a real one. An origin that is off all
monitors is never stored as home — that is a parked position, and storing it would strand the window there.

Crash recovery hangs off `PARK_HOME`, which exists only while parked: a fresh daemon that finds it
(`adopt_stale_park`) knows the previous daemon died with the window off-screen and moves it back. A clean exit
does the same through `restore_on_exit`, which disarms both hooks first (otherwise its own re-minimize
re-parks the window it is restoring) and re-minimizes at the right spot via `place_minimized_at` — minimize
first, fix `rcNormalPosition` by delta after, so the animation never plays at the parked coordinates.

Parking raises a `WindowParked` toast once per park (2 s cooldown), replacing the older "minimized windows are
not supported" message.

## Stale activation

Also in `src/daemon/window_state.rs`. A relaunched GPG window can keep the active window of its input queue after the
foreground has moved to another app. Because it still counts itself as active, clicking it back to the
foreground produces no activation, so it never rebuilds the path that hands clicks to the guest: MAA's
`PostMessage` input and the user's own clicks are both dropped while capture keeps working, since the window
still composes. Clicking *away* and back is the manual cure — the click away is what finally deactivates it.

`ActivationWatch` polls on the maintenance tick and treats "active window set while the foreground is
elsewhere" as the signature. A genuine deactivation passes through that state for ~90 ms, so only one that
outlives `ACTIVATION_REPAIR_DELAY` (500 ms) is repaired, at most three times. The repair attaches the daemon's
thread to GPG's input queue (`AttachThreadInput`) and calls `SetActiveWindow` on a hidden helper window, so the
kernel delivers the missed deactivation. The foreground is never touched, so nothing moves on screen.
