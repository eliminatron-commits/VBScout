# Third-party test files

The files in `negative/office/real/` and `positive/office/real/` were made by Microsoft Office and come unchanged from
the test data of three open-source projects. They test the Office and Access readers against files the collector's
authors did not write. `office_fixtures.py` downloads them with a pinned SHA-256 (list `REAL`); nothing else in the
test collections comes from third parties, apart from the containers named below.

| Files | Project | Source | Licence |
|---|---|---|---|
| `poi-SimpleMacro.xls`, `poi-SimpleMacro.xlsm`, `poi-SimpleMacro.doc`, `poi-SimpleMacro.docm`, `poi-SimpleMacro.pptm`, `poi-60158.docm`, `poi-60279-offset.doc`, `poi-Simple.xlsb`, `poi-password.xls`, `poi-xor-encryption-abc.xls` | Apache POI | [`test-data/`](https://github.com/apache/poi/tree/trunk/test-data) (`spreadsheet/`, `document/`, `slideshow/`) | [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) |
| `poi-59830-modules.xls`, `poi-60273-mac.xls` | Apache POI (from the govdocs1 corpus: 609751.xls, 147240.xls) | as above | Apache License 2.0 (govdocs1: documents of US government web sites) |
| `oletools-encrypted.*` | oletools (Philippe Lagadec and contributors) | [`tests/test-data/encrypted/`](https://github.com/decalage2/oletools/tree/master/tests/test-data/encrypted) | [BSD 2-Clause](https://github.com/decalage2/oletools/blob/master/LICENSE.md) |
| `jackcess-testV1997.mdb` | Jackcess | [`src/test/data/V1997/`](https://github.com/jahlborn/jackcess/tree/master/src/test/data) | [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0) |

Generated files start from some of these as containers: the Excel, Word and PowerPoint cases in `positive/office/` and
`negative/office/` are copies of the Apache POI files above with their VBA project replaced, and the Access databases
are created by Jackcess from the empty databases (made by Access) that ship with it. The libraries that write them –
Apache POI and Jackcess (Apache License 2.0) – are used only by `office_fixtures.py`, never by the collector.

## Notices

Apache POI (`legal/NOTICE`): "Apache POI – Copyright 2003-2026 The Apache Software Foundation. This product includes
software developed at The Apache Software Foundation (https://www.apache.org/)."

oletools (`LICENSE.md`): The python-oletools package is copyright (c) 2012-2025 Philippe Lagadec
(http://www.decalage.info). All rights reserved.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
following conditions are met:

* Redistributions of source code must retain the above copyright notice, this list of conditions and the following
  disclaimer.
* Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following
  disclaimer in the documentation and/or other materials provided with the distribution.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES;
LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS
SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.
