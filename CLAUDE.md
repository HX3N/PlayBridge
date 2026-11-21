# PlayBridge

PlayBridge lets MAA drive Arknights in Google Play Games on PC (GPG) by impersonating `adb` (`PlayBridgeADB.exe`,
`src/main.rs`) and MuMu's `external_renderer_ipc.dll` (`src/lib.rs`). GPG is never patched: frames come from Windows
Graphics Capture (WGC) and input is Win32 `PostMessage`. Windows only. There are no automated tests, so compile/lint
passing says nothing about runtime behaviour.

## Rules that break silently

- The DLL runs inside MAA with `panic = "abort"`, so every export must be panic-free. A failed capture is not an ABI
  error: it fills a black frame and returns `0`.
- `src/shared.rs` is the one definition of the values the EXE and the DLL must agree on (the 1280x720 display contract).
- PlayExtras frames are written bottom-up because MAA flips them; RawByNc frames are top-down, last alpha byte `0xFF`.
- Unsupported screencap modes and probes with nothing to fake are answered with silence; `Unknown` raises a toast.
- The minitouch daemon never exits on a missing window: MAA starts it before the game window may exist.
- The WGC and launcher daemons are detached and learn MAA only from the `MAA_PID` the `devices` shim publishes.
  `ensure_launcher` tests with `OpenMutexW`, because `CreateMutexW` would make the caller the owner.

## Deliberate behaviour that looks wrong

- Stale frames are served up to 30 s while the child window lives: a static GPG overlay stalls WGC on the true screen.
- GPG re-emits old frames among live ones, so frames are judged against a sampled hash history. Idle capture takes
  bursts of 3 frames because GPG's stale slot cycles every 3.
- The crop pads DWM's transparent right/bottom edge by replicating pixels; clamping shifts the UI after a resize.
- The input lock (`EnableWindow(false)`) is one-sided: posted messages skip the disabled check. A user's held capture
  is released with `AttachThreadInput` + `ReleaseCapture`, and the lock is released by idle time, not by UP.
- Before a new press, input waits while GPG's UI thread is in a move/size loop, menu tracking, or holding capture. An
  unreadable thread state counts as idle so the gate can never block MAA for good.
- Minimize is faked by parking the window below the virtual desktop. Never park with `SetWindowPlacement`, and move
  the window while it is still restored: an iconic window ignores `SetWindowPos`.
- Stale activation is repaired with `SetActiveWindow` on a hidden helper; the foreground is never touched.
- The launcher fires the launch URI at most 3 times, 45 s apart; with no GPG `store.db` record it is `GameNotInstalled`.

## Development

- Verify: `cargo fmt --check`, `cargo clippy -- -D warnings` (CI runs both), `cargo build --release`; never hand-format.
- Runtime checks need MAA + GPG: put `tools/test.bat` in the MAA folder and run it, then check connect, screencap mode,
  tap/swipe, parking with capture continuing, daemons exiting after MAA, and `debug\PlayBridge.log`.
- Never put `[release]` in a commit message unless asked: a push to `main` with it publishes a release.
- `Setup.bat` downloads `tools/Install.bat` from `main`, so a change to it reaches users as soon as it is pushed.
