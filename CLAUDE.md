# PlayBridge

PlayBridge lets [MAA](https://github.com/MaaAssistantArknights/MaaAssistantArknights) drive Arknights running in
**Google Play Games on PC (GPG)**, which MAA does not support natively. It impersonates two things MAA already speaks —
an `adb` executable and MuMu's `external_renderer_ipc.dll` — and never patches GPG: frames come from Windows Graphics
Capture (WGC) and input is Win32 `PostMessage`. Windows only (x86_64 MSVC, Rust stable).

## Scope at a glance

- Clients: YoStar KR / JP / EN. MAA's `Official`, `Bilibili`, `Txwy` have no GPG package → `UnsupportedClient` toast.
- Screencap: **PlayExtras** (the DLL) and **RawByNc** only. `RawWithGzip` / `Encode` are answered with silence (`Ignore`)
  so MAA falls back; an `Unknown` answer would raise a toast.
- Touch: **minitouch only**. `adb input tap/swipe` is refused with an `AdbInputUnsupported` toast. `input keyevent`
  (ESC only) and `input text` are posted to the game; `am force-stop` and `input keyevent HOME` close it.
- Display contract: 1280x720 everywhere (`shared.rs`); `wm size` reports it and frames are resized to it.
- No automated tests. Compile/lint passing says nothing about runtime behaviour; that needs MAA + GPG (see _Development_).

## Artifacts

| Artifact                    | Root          | Impersonates  | Runs in                                              |
| --------------------------- | ------------- | ------------- | ---------------------------------------------------- |
| `PlayBridgeADB.exe`         | `src/main.rs` | `adb`         | a new process per MAA adb call, plus three daemons    |
| `external_renderer_ipc.dll` | `src/lib.rs`  | MuMu's nemu DLL | **inside MAA's process** (PlayExtras screencap only) |

The DLL is installed as `<fakemumu>/nx_device/15.0/shell/sdk/external_renderer_ipc.dll`, with MAA's "MuMu emulator
path" set to `<fakemumu>`. Release builds use `panic = "abort"`, so a panic in the DLL kills MAA: every export must be
panic-free. A null or undersized pixel buffer returns non-zero (other pointers are trusted), but a failed capture is deliberately **not** an ABI error — it fills a
black frame and returns `0`. The nemu input exports are load-time stubs.

The bin and the cdylib are separate crates over one `src/`; each declares `mod shared;` over `src/shared.rs`, the one
definition of the values both must agree on. The EXE and DLL still ship as separate files, so a mismatched install is
possible.

## Layout

`src/` is grouped by the process the code runs in; dependencies only point down (`daemon/` → `game/` → `sys/`).

- `main.rs` — per-process setup (DPI, log rotation/depth reset, `EXE_PATH`), dispatch to a daemon or the shim
- `shim.rs` — the one-shot fake-adb call: `parse_command` / `execute_command`
- `daemon/` — `wgc` (capture daemon), `window_state` (parking, stale activation), `minitouch`, `launcher`
- `game/` — `window` (finding GPG, the user-action gate), `input` (`PostMessage`, lock primitives), `capture`
- `sys/` — registry config, logging, toasts, MAA lookup, GPG `store.db`, named mutexes
- `lib.rs` — the nemu exports

## Processes

| Process          | Started by                              | Ends when                                             |
| ---------------- | --------------------------------------- | ----------------------------------------------------- |
| shim             | MAA, once per adb call                  | the command is answered                               |
| WGC daemon       | `--wgc-daemon`, spawned by the DLL or a cold `ScreencapNc` | MAA's PID (`MAA_PID`) disappears, polled ~1 s |
| launcher         | `--launcher-daemon`, from `devices`, `am start`, or a frameless capture request | game window up, MAA gone, or 180 s |
| minitouch        | MAA's `shell <path> -i`                 | MAA closes its stdin pipe                             |

The WGC and launcher daemons are spawned `DETACHED_PROCESS`, so they cannot walk up to MAA themselves: only the
`devices` shim has MAA as its parent, and it publishes `MAA_PID` and the client. With no PID published the daemons
treat MAA as alive; a published PID that `OpenProcess` cannot open counts as gone. WGC and launcher are single-instance via named mutexes; `ensure_launcher` tests with
`OpenMutexW` because `CreateMutexW` would make the caller the owner and every later check would see a phantom daemon.

Cross-process state lives in `HKCU\Software\PlayBridge\state`:

| Key               | Meaning                                                                   |
| ----------------- | ------------------------------------------------------------------------- |
| `WGC_DAEMON_PORT` | daemon's TCP port, `0` while down (read by the shim and the DLL)          |
| `EXE_PATH`        | lets the DLL spawn `--wgc-daemon` without knowing where the bin is        |
| `MAA_PID`         | the MAA the daemons live for                                              |
| `WINDOW_HOME`     | where the GPG window belongs; survives daemon restarts                    |
| `PARK_HOME`       | present only while parked; found at startup = previous daemon died parked |
| `LOG_DEPTH`       | open log blocks across all processes                                      |

`…\config` (client, version, touch overlay, update throttle) and `…\cooldown` (per-toast timestamps) are shared by the
bin's processes but never read by the DLL.

## MAA contract

MAA's `MuMuEmulator12` connect path runs a fixed adb sequence (`devices`, `connect`, android_id, `getprop`, `wm size`,
arp, abilist, orientation, minitouch `push`/`chmod`, then `shell … -i`). Probes with nothing to fake (arp, `push`,
`chmod`, `start-server`/`kill-server`) are answered with silence rather than `Unknown`. Constraints that come from MAA:

- `devices` must list `host:port` with a port MAA's `get_mumu_index()` accepts, or MAA skips PlayExtras.
- MAA starts minitouch as soon as connect returns, possibly before the game window exists. The handshake has already
  told MAA it is up, so the minitouch daemon never exits on a missing window; it binds on the first commit that finds one.
- After connect MAA benchmarks its screencap modes and locks onto the fastest.
- **PlayExtras**: DLL → TCP `127.0.0.1:<port>`, sends port `0` (the byte-return verb), gets `[w u32][h u32][rgba]`.
  MAA applies `RGBA2BGR` then `flip(.,0)` to these frames, so the DLL writes **bottom-up** to cancel the flip.
- **RawByNc**: `exec-out screencap | nc -w 3 10.0.2.2 <port>` → the shim sends `<port>` to the daemon, the daemon
  connects out to MAA's port with `[w][h][format=1][rgba]` top-down (last alpha byte forced to `0xFF` for MAA's
  validation), then ACKs the shim. A cold daemon is spawned and that one request gets a black frame.
- Both paths converge on the **single** WGC daemon: one `handle_client` serves both, told apart by the port (`0` =
  return the frame on the same socket).

## GPG windows

```
HwndWrapper[DefaultDomain;;<guid>]   top-level, crosvm.exe, titled "<game name> - <doctor>"
  └ CROSVM_1                         input target; its client rect is the crop region
```

`GameWindow::find()` makes one pass over visible titled windows: the title must start with a client's game name
(`명일방주` / `アークナイツ` / `Arknights`) **and** the window must own a direct `CROSVM_1` child. The prefix keeps a GPG
session running another game away from MAA and identifies the client in the same step; the child check rejects GPG's
chrome. GPG also keeps four invisible 12x12 untitled top-levels with their own `CROSVM_1`, which the visible/titled
filter drops. The launch/loading window is titled exactly `Google Play Games` with an `HwndWrapper*` class; force-stop
posts `WM_CLOSE` to those when no game window exists.

WGC captures the **top-level** window and crops to the `CROSVM_1` client area per request.

## Deliberate behaviour that looks wrong

**Frames.** WGC keeps compositing occluded windows but stops on a genuine minimize (hence parking). Arknights
animates constantly, so a frame older than 1 s means composition stopped — but a static GPG overlay (user-center
webview) stalls WGC while the last frame is still the true screen, so stale frames are served for up to 30 s while the
child window lives, then black. DWM leaves the right/bottom ~3 px transparent: the crop keeps full size and pads by
replicating edge pixels (clamping shifts the UI after resize). Win11 rounded corners are squared while capturing and
restored on exit. The frame pool is fixed to the window size, so a resize needs a new session even on the same HWND;
rebinds build on a worker so the old capture keeps serving. Idling never ends the daemon; it drops to a low-power
frame rate instead of paying the GPU→CPU copy for every frame.

**Input lock (minitouch).** User clicks share the queue MAA's posted touches go into, so a stray click tears a swipe.
While a touch is in flight the top-level window is `EnableWindow(false)`: inherited by the child, and posted messages
skip the disabled check, so the lock is one-sided. It only blocks hit-tested input, though — a user already holding the
button keeps mouse capture, and capture can only be released from GPG's UI thread input state, so the watchdog
`AttachThreadInput`s and calls `ReleaseCapture`, chasing briefly because the posted press takes capture late. Release
is by idle time, not UP, so the lock survives the gap between consecutive swipes; a held long-press stops the idle clock. A daemon killed
mid-swipe leaves the window disabled, so every fresh bind re-enables it.

**User-action gate.** Before a new press only, `await_modal_end` waits while GPG's UI thread is in a move/size loop,
menu tracking, or holding capture: the first two drain mouse messages from the whole thread queue, swallowing posted
ones. The `InputHeld` toast is requested every poll; its registry cooldown spaces it out. An unreadable thread state
counts as idle, because a gate stuck closed would block MAA for good.

**Window parking.** Minimize is faked: the window stays restored and is moved just below the virtual desktop (Y only,
so the taskbar button keeps its monitor). The `MINIMIZESTART` hook must move it while it is still restored — an
iconic window silently ignores `SetWindowPos` — and the maintenance tick then un-minimizes it out of sight;
`EVENT_SYSTEM_FOREGROUND` moves it home. `SetWindowPlacement` is never used to park because it drags an off-screen
`rcNormalPosition` back onto the primary monitor. Home is learned only from an on-monitor origin (a minimized window
reports a bogus one, so it is surfaced for one pass if no home is cached). A leftover `PARK_HOME` at startup means a
crash while parked; clean exit disarms the hooks first, then minimizes before fixing `rcNormalPosition` by delta so the
animation never plays off-screen.

**Stale activation.** A relaunched GPG can keep its queue's active window after the foreground moved away; clicking it
back then produces no activation, so both MAA's and the user's clicks are dropped while capture still works. "Active
window set, foreground elsewhere" is the signature; a normal handover passes through it for ~90 ms, so only one
lasting 500 ms is repaired (max 3 tries) by attaching to GPG's queue and `SetActiveWindow` on a hidden helper. The
foreground is never touched.

## Client resolution and launch

1. `devices` proposes the client from `<MAA>/config/gui.new.json` → `Configurations[Current].Gui.RuntimeSettings.ClientType`.
   Unsupported types store nothing; an unreadable config leaves it to the window title.
2. A running window whose title names another client overrides it (`adopt_running_client`).
3. With GPG closed, the launcher checks GPG's `store.db` for the package; on a miss it adopts the one other client
   that is installed (two installed side by side is not guessed at).

`CLIENT` is written before any `ClientMismatch` toast because toast language follows it. A corrected client makes
MAA's next `am start` intent contradict it, which raises the mismatch on its own.

Launching is its own short-lived process whose lifetime *is* the "game is starting" signal. Without a `store.db`
record the launch URI only raises GPG's own window (endless loading), so a miss with no other client to adopt becomes
`GameNotInstalled`. It fires
`googleplaygames://launch/?id=<package>&pid=1`, never while a loading window is up; with neither loading nor game
window it retries every 10 s, and a loading window that vanishes without the game resets that cooldown. Only
`devices`, `am start`, and a capture request that finds no frame start it, so closing GPG after MAA finishes does not
bring it back. `store.db` (SQLite, protobuf BLOB) is read raw with a shared read while GPG runs; the layout is in
`sys/store.rs`. Its current render resolution is checked on every bind and warned about (non-16:9, or 16:9 but not 1280x720) when it
differs from the last value seen.

## Logging

`<exe dir>\debug\PlayBridge.log` (size-rotated at process start) is a merged timeline of every shim call and daemon.
Block nesting (`┌`/`└`) is a depth stored in the registry, not per process, so a block one process holds open (each
daemon's lifetime) indents what the others write; updates go through a named mutex, where an abandoned mutex counts as
acquired. A killed process strands the depth, so `main()` resets it when neither the WGC nor the launcher mutex exists
(minitouch has no mutex, so its block can be flattened by that reset).

## Development

- Verify: `cargo fmt --check`, `cargo check`, `cargo clippy -- -D warnings` (CI runs these on Windows), and
  `cargo build --release`. Formatting follows `rustfmt.toml` (140 columns) — run `cargo fmt`, don't hand-format.
- MAA settings (Settings > Connection): ADB path `PlayBridgeADB.exe`, address `127.0.0.1:6000`, preset MuMu Emulator,
  touch mode Minitouch, MuMu path `PlayExtras`, screenshot enhancement enabled.
- Live test: put `tools/test.bat` in the MAA folder (it checks for `MAA.exe`) and run it; it copies
  `%USERPROFILE%\Documents\GitHub\PlayBridge\target\release` artifacts into `PlayBridgeADB.exe` and
  `PlayExtras\nx_device\…`, waiting while MAA holds them. Then check: connect, screencap mode chosen, tap/swipe,
  minimize (parking) with capture continuing, daemons exiting after MAA, and the log. `PlayBridgeADB.exe --touch-overlay` toggles
  per-touch PNGs under `debug\PlayBridge\` for checking coordinates.
- **Release**: a push to `main` whose head commit message contains `[release]` (or a manual dispatch) builds and
  publishes a GitHub release with `PlayBridgeADB.exe`, `external_renderer_ipc.dll`, and `tools/Setup.bat`. Never put `[release]` in a commit message unless asked. The version (`vYYYY.MM.DD_HH.mm`,
  KST) is injected via `PLAYBRIDGE_VERSION`; local builds report `development` and skip the update check.
- Users install with `Setup.bat`, which downloads `tools/Install.bat` **from `main`**, which then downloads the latest
  release binaries — a change to `Install.bat` reaches users as soon as it is pushed, without a release.
