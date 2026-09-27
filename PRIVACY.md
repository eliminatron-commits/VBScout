# Privacy notice – VBScout software

*As of 2026-09-27. This notice covers the programs (collector and evaluation app) and the license purchase. The
website has its own notice. Controller: the vendor named on the website (imprint).*

## In short

The collector and the evaluation app work **completely offline**: they send nothing to the vendor or anyone else –
no telemetry, no crash reports, no update checks, no online activation. What they read and produce stays where you
put it. Automated tests block and log every network connection attempt of both programs; the release is only built
when they record none.

## Collector (`<Name>-Collector-<version>-x64.exe`)

The collector reads the computer it runs on (files, registry, scheduled tasks, services, WMI subscriptions, installer
packages, event logs, Office documents) **read-only** and writes exactly one result file (`.vbscout`) to the folder
you choose. The result file contains:

* **The machine**: host name, fully qualified name and DNS domain (from the local configuration – no directory
  query), Windows version and edition, a pseudonymous machine ID (SHA-256 hash of the Windows `MachineGuid`, so the
  same computer is recognised again without storing the GUID itself), time of the scan, collector version, and
  whether it ran with administrator rights.
* **Findings**: paths of affected files and entries (these can contain user names, e.g. `C:\Users\<name>\…`), names
  of scheduled tasks, services and autostart entries, file size and SHA-256 hash, and **short excerpts of the
  affected lines only**. Passwords, connection strings and similar secrets are masked before the file is written;
  their values are never stored.
* **Coverage**: which sources could be read, and why others could not.

The result file is not encrypted. Treat it like other inventory data: store it on a share with suitable permissions
and delete it when the evaluation is done. If you name a network folder for the result (`--out`) or a network path to
scan (`--include-unc`), Windows' file system accesses it with the rights of the running account; the collector itself
opens no network connections.

## Evaluation app

The app reads the result files you open and keeps them in memory while it runs; nothing is uploaded. It stores on
the computer, in the user's application data folder: its settings (language, customer name for reports), the logo
you choose for reports, and your license key. Reports (PDF, Excel) are written only where you save them. The
license key is checked on the computer; it contains the licensee name (organization or company), the license type,
dates and a key ID.

## Buying a license

Licenses are sold by **Paddle** (Paddle.com Market Ltd.) as merchant of record: Paddle processes the order, payment,
invoice and taxes under its own privacy notice (paddle.com/legal). The vendor's key service receives Paddle's
notification of the completed purchase and stores, per purchase: the Paddle transaction, customer and subscription
IDs, the license type, the licensee name you entered, the dates and the issued key. To send the key by e-mail it
reads your e-mail address from Paddle and hands it to the e-mail provider for that one message; the address is not
stored by the key service. Legal basis: performance of the contract (Art. 6(1)(b) GDPR). Purchase records are kept as
long as commercial and tax law requires.

## Your rights

You can request access, correction, deletion, restriction or transfer of your data, and object to processing; you
can complain to a supervisory authority. Contact: the support address on the website.
