# sysreceipt

Windows binary wrapper for [receipt](https://github.com/akshaytheflash/receipt).

```
npx sysreceipt run -- installer.exe /S
```

Or install globally:

```
npm install -g sysreceipt
receipt run -- installer.exe /S
```

`receipt` records what an installer changes on Windows (files, registry,
services, drivers, scheduled tasks, network, processes) and can revert it.

Windows x64 only.
