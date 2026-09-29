# network-speed-gui

A Windows desktop app for running [iperf3](https://software.es.net/iperf/) TCP throughput
tests without a terminal. It shows throughput live (a number and a line plot), summarizes the
result, and saves the output to a log file.

## Requirements

- Windows 10 or 11
- Rust 1.95+ (MSVC toolchain)
- iperf3 3.5 or newer

## Build and run

```powershell
cargo test                 # parser unit tests
cargo run                  # debug build
cargo build --release      # target\release\network-speed-gui.exe
.\target\release\network-speed-gui.exe   # run the release build
```

## Installer

Requires:
- [cargo-wix](https://github.com/volks73/cargo-wix) (`cargo install cargo-wix`)
- [WiX Toolset v3](https://github.com/wixtoolset/wix3/releases), which cargo-wix uses to
  build the MSI. Its `bin` folder must be on `PATH`, or the `WIX` environment variable must
  point to the toolset folder.

1. Build the release
2. `cargo wix` builds `target\wix\network-speed-gui-0.1.0-x86_64.msi`.
3. Run the `.msi`.

It installs to `Program Files\network-speed-gui` and adds an **Iperf3 Network GUI Tester**
Start Menu shortcut. To uninstall, use *Settings → Apps → Installed apps* (listed as
"Iperf3 Network GUI Tester"), or re-run the `.msi` and choose **Remove**.

## iperf3

**Tested with iperf3 3.5** . Later versions are backwards compatible on input
and output, so supporting the oldest allowed version was the priority.

**Not bundled.** The app is an extension of iperf3, not a bundle of it. Users may want to
test with a specific version, which a bundled copy might not match. The user installs the
iperf3 they want. The app searches `PATH` at startup and checks `--version` is 3.5 or newer.
If none is found, a popup asks for the path, which can be typed or browsed at any time.

## Framework and architecture

- **Rust:** chosen for its memory safety, and it's my preferred language.
- **egui:** its docs and forums specified that it allows quick building and prototyping.
- **cargo-wix:** MSI is Windows' native installer format, and cargo-wix documents the steps
  for building one with Cargo.

The code is split into two modules.

**`src/iperf/`: running iperf3**
- `config.rs` validates the host, port and duration and builds the iperf3 arguments.
- `locate.rs` finds iperf3 on `PATH`, or checks a user-chosen path, on a background thread.
- `runner.rs` starts iperf3. Two reader threads (stdout and stderr) feed a worker thread,
  which parses each line and sends `Interval` events, then one `Ended` event with the
  outcome and the full output.
- `parser.rs` turns output lines into measurements (see below).
- `job.rs` puts iperf3 child process in a Windows Job Object, so it's killed even if the GUI crashes. Terminate kills it directly.

**`src/app/`: the egui GUI**
- egui redraws the window from the app's state every frame. Each frame, the app checks for
  new events without blocking, so the UI never freezes. The worker wakes it with
  `request_repaint` when something arrives.
- `RunState` (`Idle → Running → Ended`) holds everything about the current or last test.
- There are three panels:
  - **Setup:** the config inputs and Start/Terminate.
  - **Live:** the latest throughput and a chart of the last 30 intervals.
  - **Result:** the summary or error, and **Save log**.

**Saving results:** a test is saved as iperf3's exact output in a `.log` file, so users can
debug themselves or look at an error more closely. A header adds test information that
iperf3's output doesn't include: the iperf3 version, its path, and the command run.

## Parsing iperf3 output

The app runs `iperf3 -c <host> -p <port> -t <seconds> -i 1 --forceflush`. `--forceflush`
stops Windows iperf3 from buffering piped output.

JSON streaming isn't available in iperf3 3.5, so the app parses the stdout text with a
simple line parser. It collects the intervals, the summary rows and error
lines, and ignores everything else, using a small state machine:

```text
Preamble ──"[ ID]" header──▶ Intervals ──"- - -" separator──▶ Summary
```

A regex picks out data rows (`[<number>] ...`). In `Summary`, only rows ending in `sender`
or `receiver` are kept.

## Error handling

| Situation | Behaviour |
|---|---|
| iperf3 not on `PATH` | Popup asking for the path |
| Bad path, not iperf3, version < 3.5, or `--version` times out | Error under the path field; Start disabled |
| Invalid host, port or duration | Hint shown; Start disabled (port and duration accept digits only) |
| iperf3 can't start, or a Cygwin DLL is missing | "could not start iperf3" with the reason |
| iperf3 reports an error (e.g. connection refused) | iperf3's error message |
| iperf3 exits with a non-zero code | Exit code and the last output lines |
| Cancel, or GUI closed or crashed mid-test | iperf3 is killed; no process left running |
| Saving the log fails | The reason, next to the Save button |

## Known limitations

- Only TCP client tests, with a single stream sent from this machine to the server (no UDP,
  reverse `-R`, or parallel `-P`).
- No retransmit count on Windows (see Ambiguities).
- Assumes one-second intervals. The "Interval N of ~D" status and the 30-point chart window
  rely on it.
- Settings aren't saved, so a custom iperf3 path, host and port must be entered on every
  launch.

## Ambiguities

- **Retransmits "where the platform reports them":** iperf3 can't report retransmits on
  Windows ([esnet/iperf#1957](https://github.com/esnet/iperf/discussions/1957)), so the
  summary shows N/A.
