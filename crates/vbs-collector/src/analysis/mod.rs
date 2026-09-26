//! Format readers and detectors shared by the modules. They work on bytes and text only –
//! no file access, no operating-system calls – so they are easy to test anywhere.
//!
//! * [`text`] – decoding of script files (UTF-8/UTF-16/ANSI) and lines.
//! * [`command`] – VBScript in command lines (Script Host, direct starts, `vbscript:`).
//! * [`markup`] – lenient XML/HTML scanner (.wsf, .wsc, .hta, task definitions).
//! * [`vbe`] – decoder for scripts encoded with the Script Encoder.
//! * [`lnk`] – shell links (.lnk).
//! * [`msi`] – installer packages (.msi), read as compound files.
//! * [`ini`] – Group Policy script lists (scripts.ini, psscripts.ini).
//! * [`event_xml`] – events as the Windows Event Log API renders them.
//! * [`regf`] – registry hive files (`NTUSER.DAT` of users who are not logged on).
//! * [`script`] – lines of scripts: comments, evidence selection.

pub mod command;
pub mod event_xml;
pub mod ini;
pub mod lnk;
pub mod markup;
pub mod msi;
pub mod regf;
pub mod script;
pub mod text;
pub mod vbe;
