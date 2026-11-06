# Windows crash and hang diagnostics

No Rust, C++, or Qt development environment is required on the affected machine.
Use GitHub Actions to build a runnable package with matching application symbols.

## Build and download

1. Push the changes you want to investigate, including the workflow, to GitHub.
   GitHub requires a `workflow_dispatch` workflow to exist on the default branch
   before its **Run workflow** button is available.
2. Open **Actions → Windows diagnostics → Run workflow** and select the branch
   containing the code to investigate. This does not create a GitHub Release.
3. Once the run succeeds, download the
   `tailer-windows-diagnostics-<run-id>-<attempt>` artifact from the run's page.
4. Extract the entire ZIP into a new directory, for example `C:\TailerDebug`.
   Do not mix DLLs or symbols from different builds.
5. Ensure the Microsoft Visual C++ v14 x64 Redistributable is installed, then run
   `C:\TailerDebug\tailer.exe`. See the runtime prerequisite below.

This is an **optimized release build with debug information**, not an unoptimized
debug build. It uses the normal release profile and the same Qt version as the
Windows release workflow to minimize changes to the reproduction conditions.
Qt DLLs, plugins, and QML modules are included. The package is not Authenticode
signed.

### Windows runtime prerequisite

The affected machine does not need development tools, but it does need the
Microsoft Visual C++ v14 x64 runtime. `windeployqt --compiler-runtime` may bundle a
Redistributable **installer**, rather than app-local runtime DLLs. Extracting the
ZIP does not run that installer. If the runtime is missing, install the bundled
x64 Redistributable or obtain it from
[Microsoft](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).
Follow the machine's installation policy; administrator approval may be needed.
Missing `VCRUNTIME140*.dll` or `MSVCP140*.dll` at startup indicates this prerequisite
may not be satisfied. Do not download individual DLLs from third-party sites.

### Preserve the build

Keep the whole package, especially `tailer.exe`, `tailer.pdb`, the Qt DLLs,
`BUILD-INFO.txt`, and `SHA256SUMS.txt`, together with any dump. The artifact expires
after 30 days (or sooner if repository policy or manual deletion removes it).
Normal Windows release ZIPs also include their corresponding `tailer.pdb`.

## Capture a crash or an unresponsive window

For a crash, use Windows Error Reporting LocalDumps or a debugger to collect a
dump. For a hang, leave the process running, open Task Manager's **Details** tab,
right-click `tailer.exe`, and choose **Create memory dump file**. The wording may
vary by Windows version. Task Manager displays the saved file's location.

**Dumps can contain SSH passwords, credentials, and log contents. Do not upload
them to public issues or public artifacts.** Preserve them only in a trusted
location, following the machine's security policy. Do not disable security
software to collect a dump.

## Load application symbols in WinDbg

Open the dump in WinDbg. If the matching package was extracted to
`C:\TailerDebug`, run:

```text
.symfix
.sympath+ C:\TailerDebug
.reload /f tailer.exe
lmvm tailer
```

The module should report PDB symbols, rather than only `(export symbols)`.
If it still shows exports, diagnose the symbol search with:

```text
!sym noisy
.reload /f tailer.exe
!sym quiet
```

Do not force a mismatched PDB to load. A newly rebuilt PDB cannot normally be used
with an older EXE's dump, even if both builds came from the same source revision.
If the old PDB is unavailable, reproduce using the new EXE and collect a new dump.

For a **crash** dump, inspect the exception and its thread:

```text
!analyze -v
.ecxr
kv
```

For a **hang** dump, inspect all threads, not just the currently selected thread:

```text
~* kv
!analyze -hang
```

Copying output from a remote desktop is optional. A screenshot of the relevant
stack and the `lmvm tailer` symbol status can also be useful. Share
`BUILD-INFO.txt` to identify the build without sharing a sensitive dump.

## Symbol limitations

`tailer.pdb` provides application symbols; this workflow does **not** obtain Qt's
matching PDBs. Microsoft's symbol server helps with Windows system modules, not
Tailer's private symbols or necessarily Qt's symbols. Qt frames may still show
only exported names until the exact Qt build's PDBs are provided separately.

With only export symbols, a frame such as
`tailer!aws_lc_...+0x12e42` may refer to the nearest exported symbol rather than the
actual Rust function. That name alone does not identify AWS-LC as the cause.
Similarly, `0xC0000409` with subcode `FAST_FAIL_FATAL_APP_EXIT` indicates fatal
termination; it is not by itself proof of a stack-buffer overwrite or security
software intervention.