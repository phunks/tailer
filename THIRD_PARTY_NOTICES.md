# Third-party notices

Tailer's original project code is Copyright (c) 2026 pnk and is licensed under
MIT; see `LICENSE`. Third-party components retain their own copyrights and
licenses. This document is not a complete third-party license inventory.

## Qt

Tailer uses Qt Core, GUI, QML, Quick, and related Qt Quick modules. macOS and
Windows packages include Qt runtime libraries and QML plugins. Qt is developed
by The Qt Company Ltd. and other contributors.

The open-source license selection for LGPL-eligible Qt components is LGPLv3.
Some Qt modules have different terms, including GPL-only modules; verify every
deployed module before publishing a package. Tailer's MIT license does not
override these terms.

The supplied `licenses/Qt/LICENSE` contains the GPLv3 and LGPLv3 texts and Qt's
licensing overview. Other supplied files in that directory contain additional
license material, including CMake notices and documentation licensing; they
must not be mistaken for a complete notice inventory for the Qt runtime.

- Licensing: https://doc.qt.io/qt-6/licensing.html
- Upstream source archive index: https://download.qt.io/archive/qt/

These references identify upstream resources, not a complete corresponding-source
offer. Distributors must provide sources matching the actual shipped binaries,
including modifications, using a method permitted by the applicable licenses.

## Qt Bridge for Rust

Copyright (C) 2025 The Qt Company Ltd. (as stated in upstream source headers).
The Qt Bridge workspace declares
`LicenseRef-Qt-Commercial OR LGPL-3.0-only`; this project's open-source license
selection is LGPL-3.0-only. See `licenses/QtBridge/LGPL-3.0-only.txt` and the
GPLv3 text in `licenses/Qt/LICENSE`.

`licenses/QtBridge` preserves the complete upstream `LICENSES` directory:
LGPL-3.0-only, MIT, Apache-2.0, BSD-3-Clause, and LicenseRef-Qt-Commercial.
These documents apply according to each source file's license designation;
their presence does not mean all licenses apply simultaneously. Some crate
source files are designated `MIT OR Apache-2.0`. The commercial-license
reference is retained as upstream documentation, not as a claim that this
distribution holds or selects a commercial Qt license.

The upstream MIT and BSD documents contain template copyright fields. Actual
copyright notices must be retained from the relevant source files; copying
these license texts alone is not a complete copyright-notice inventory.
The `qt-version` and `qt-artifacts` crate source headers state Copyright (C)
2026 The Qt Company Ltd. and `MIT OR Apache-2.0`.

- Repository: https://github.com/qt/qtbridge-rust
- Revision pinned by `just setup-ext`: `f7fe523d10357751bc90c334263188610c0382cf`

Qt Bridge code is incorporated into the executable. LGPL obligations include
enabling users to rebuild/relink the application with a modified version of the
library. Tailer's MIT-licensed source, `Cargo.lock`, and build scripts may form
part of that process, but distributors must also provide matching library
sources and usable build/install instructions. Any changes to the pinned
dependency must be included in the corresponding sources.

## Other dependencies and redistribution

Rust dependencies and third-party components inside Qt have their own license
and notice requirements. Their complete inventory and corresponding-source
delivery remain release preparation tasks; this file does not certify that
those obligations have been satisfied.

LGPL-covered library modification and debugging of those modifications must
not be prohibited. Distribution must preserve applicable library replacement,
relinking, and installation rights. macOS packages are ad-hoc signed; modified
binaries/libraries may need to be re-signed for local execution.