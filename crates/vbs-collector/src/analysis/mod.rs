//! Format readers and detectors shared by the modules. They work on bytes and text only –
//! no file access, no operating-system calls – so they are easy to test anywhere.
//!
//! * [`text`] – decoding of script files (UTF-8/UTF-16/ANSI) and lines.
//! * [`command`] – VBScript in command lines (Script Host, direct starts, `vbscript:`).
//! * [`markup`] – lenient XML/HTML scanner (.wsf, .wsc, .hta, task definitions).
//! * [`vbe`] – decoder for scripts encoded with the Script Encoder.
//! * [`lnk`] – shell links (.lnk).
//! * [`msi`] – installer packages (.msi), read as compound files.
//! * [`ovba`] – VBA projects: decompression, `dir` and `PROJECT` streams, module source code.
//! * [`office`] – Office documents by content: compound files, Office Open XML packages and
//!   their embedded objects, encrypted packages; [`jet`] – Access databases (Jet/ACE).
//! * [`ini`] – Group Policy script lists (scripts.ini, psscripts.ini).
//! * [`event_xml`] – events as the Windows Event Log API renders them.
//! * [`regf`] – registry hive files (`NTUSER.DAT` of users who are not logged on).
//! * [`script`] – lines of scripts: comments, evidence selection.
//! * [`vba`] – VBScript in VBA code and references (RegExp, Script Control, script starts, WSH).
//! * [`servicing`] – Windows servicing data (component store differentials and compressed
//!   payloads) that only carries the name of a script, shortcut, package or document.

pub mod command;
pub mod event_xml;
pub mod ini;
pub mod jet;
pub mod lnk;
pub mod markup;
pub mod msi;
pub mod office;
pub mod ovba;
pub mod regf;
pub mod script;
pub mod servicing;
pub mod text;
pub mod vba;
pub mod vbe;
