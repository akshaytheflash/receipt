# receipt

Run an installer, see what it changed, undo it. Windows only.

Most installers do not tell you what they did. `receipt` runs the command under
a watcher, records files, registry, services, drivers, scheduled tasks, network
connections and processes, then writes a manifest you can inspect or revert.

It is a recorder, not a sandbox. The program runs with your normal permissions
and really changes the machine.

## Install

```
winget install akshaytheflash.receipt
```

```
scoop install receipt
```

```
npm install -g sysreceipt
```

Or download `receipt-x86_64-pc-windows-msvc.exe` from the releases page and put
it somewhere on `PATH`. Every release publishes `SHA256SUMS.txt`.

## Use

```
receipt run -- installer.exe /S
```

Output from a real run:

```
  receipt 0.1.0

  session    20261101-092343-ca90
  command    .\target\release\fixture_installer.exe C:\Users\aksha\AppData\Local\Temp\receipt-demo
  exit       0
  elevated   no
  time       6.9s

  changes
  files      5 created
  registry   1 keys   2 values
  services   -
  tasks      -
  drivers    -
  network    2 connections
  processes  3 started

  created files
    C:\Users\aksha\AppData\Local\Temp\receipt-demo
    C:\Users\aksha\AppData\Local\Temp\receipt-demo\alpha.txt
    C:\Users\aksha\AppData\Local\Temp\receipt-demo\beta.txt
    C:\Users\aksha\AppData\Local\Temp\receipt-demo\nested
    C:\Users\aksha\AppData\Local\Temp\receipt-demo\nested\gamma.txt

  created registry keys
    HKCU\SOFTWARE\ReceiptFixture

  created registry values
    HKCU\SOFTWARE\ReceiptFixture\Alpha = one
    HKCU\SOFTWARE\ReceiptFixture\Beta = 42

  network connections
    tcp  127.0.0.1:56072 -> 127.0.0.1:47821  (fixture_installer.exe)
    tcp  127.0.0.1:47821 -> 127.0.0.1:56072  (fixture_installer.exe)

  warnings
    not elevated: HKLM writes and service changes are not visible

  undo with receipt revert 20261101-092343-ca90
```

Undo:

```
receipt revert 20261101-092343-ca90
```

```
  files 3   dirs 2   registry 1 keys / 2 values / 0 restored   services 0   tasks 0
  clean
```

`clean` means every path and key the session created is gone.

## Commands

```
receipt run [--name X] [--json] [--timeout SECS] [--watch DIR] [--no-network] [--no-processes] -- CMD ARGS...
receipt list [--json]
receipt show [ID] [--json]
receipt revert [ID] [--yes] [--dry-run] [--kill]
```

Put `receipt`'s own flags before `--`. Everything after `--` is the child command.

`show` and `revert` default to the most recent session. Sessions live in
`%LOCALAPPDATA%\receipt\sessions\<id>\session.json`.

`--dry-run` prints what revert would delete and changes nothing. `--kill` also
terminates processes whose executable sits under a created path.

## What is recorded

| Area | Method |
| --- | --- |
| Files | `ReadDirectoryChangesW` on Program Files, Program Files (x86), ProgramData, `%LOCALAPPDATA%\Programs`, `%LOCALAPPDATA%\Temp`, `%APPDATA%`, `%SystemRoot%\Temp`, plus `--watch` |
| Registry | Before/after snapshot of Run, RunOnce, Uninstall, App Paths, CLSID, Services, FirewallRules, and `HKCU\SOFTWARE` |
| Services | New or changed keys under `HKLM\SYSTEM\CurrentControlSet\Services` |
| Drivers | Same key, kernel driver types only |
| Scheduled tasks | New files under `C:\Windows\System32\Tasks` |
| Network | Polled every 400ms, new remote endpoints only |
| Processes | Polled every 400ms, new PIDs only |

File contents are not copied. Registry values are.

## Revert

Revert does this:

- deletes files and empty directories created during the run
- deletes registry keys and values created during the run
- restores registry values that were changed
- stops and deletes services created during the run
- deletes scheduled task files created during the run

Revert does not do this:

- restore the previous contents of a file that was overwritten
- remove drivers (they are listed and left alone; removing a driver by hand can
  break boot)
- undo network connections or kill processes unless you pass `--kill`
- attribute a change to a specific child process; everything is session-level

Revert refuses to delete anything under `System32`, `SysWOW64`, `WinSxS`,
`Windows\Servicing`, `System Volume Information` or `$Recycle.Bin`, even if a
manifest asks for it.

Run `receipt` from an elevated shell if you want HKLM writes, services and
drivers recorded. Unelevated runs still record files, HKCU and processes, and
print a warning.

## Build

```
cargo build --release
cargo test
```

Requires Rust 1.85 or newer. Windows only; there is no Linux or macOS support.

## License

MIT
