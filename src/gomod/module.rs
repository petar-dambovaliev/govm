#![allow(dead_code)]

use crate::gomod::semserver;
use chrono::NaiveDateTime;
use chrono::{DateTime, TimeZone, Utc};
use glob::Pattern;
use lazy_regex::regex;
use rust_decimal::prelude::*;
use std::error::Error;
use std::fmt;
use std::ops::Add;

// The Version struct is defined by a module path and version pair.
// These are stored in their plain (unescaped) form.
#[derive(Debug)]
pub struct Version {
    path: String,
    version: String,
}

impl Version {
    // String method returns a representation of the Version suitable for logging
    // (Path@Version, or just Path if Version is empty).
    fn string(&self) -> String {
        format!("{}@{}", self.path, self.version)
    }
}

// A ModuleError indicates an error specific to a module.
#[derive(Debug)]
struct ModuleError {
    path: String,
    version: Option<String>,
    err: Box<dyn Error>,
}

impl ModuleError {
    // VersionError returns a ModuleError derived from a Version and error,
    // or err itself if it is already such an error.
    fn version_error(v: Version, err: Box<dyn Error>) -> Box<dyn Error> {
        match &err.downcast_ref::<InvalidVersionError>() {
            Some(m_err) if m_err.version == v.version => err,
            _ => Box::new(ModuleError {
                path: v.path,
                version: Some(v.version),
                err,
            }),
        }
    }
}

impl std::fmt::Display for ModuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(v) = &self.version {
            write!(f, "{}@{}: {}", self.path, v, self.err)
        } else {
            write!(f, "module {}: {}", self.path, self.err)
        }
    }
}

impl Error for ModuleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.err)
    }
}

// An InvalidVersionError indicates an error specific to a version, with the
// module path unknown or specified externally.
//
// A ModuleError may wrap an InvalidVersionError, but an InvalidVersionError
// must not wrap a ModuleError.
#[derive(Debug)]
struct InvalidVersionError {
    version: String,
    pseudo: bool,
    err: Box<dyn Error>,
}

impl InvalidVersionError {
    // noun returns either "version" or "pseudo-version", depending on whether
    // e.Version is a pseudo-version.
    fn noun(&self) -> &'static str {
        if self.pseudo {
            "pseudo-version"
        } else {
            "version"
        }
    }
}

impl std::fmt::Display for InvalidVersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {} invalid: {}", self.noun(), self.version, self.err)
    }
}

impl Error for InvalidVersionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&*self.err)
    }
}

// An InvalidPathError indicates a module, import, or file path doesn't
// satisfy all naming constraints.
#[derive(Debug)]
pub struct InvalidPathError {
    kind: &'static str, // "module", "import", or "file"
    path: String,
    err: Box<dyn Error>,
}

impl InvalidPathError {
    fn new(kind: &'static str, path: String, err: Box<dyn Error>) -> Self {
        InvalidPathError { kind, path, err }
    }
}

impl std::fmt::Display for InvalidPathError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "malformed {} path {}: {}",
            self.kind, self.path, self.err
        )
    }
}

impl Error for InvalidPathError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.err)
    }
}

fn split_gopkg_in(path: &str) -> (String, String, bool) {
    if !path.starts_with("gopkg.in/") {
        return (path.to_string(), "".to_string(), false);
    }

    let mut i = path.len();

    if path.ends_with("-unstable") {
        i -= "-unstable".len();
    }

    while i > 0 && path.chars().nth(i - 1).unwrap().is_digit(10) {
        i -= 1;
    }

    if i <= 1 || path.chars().nth(i - 1).unwrap() != 'v' || path.chars().nth(i - 2).unwrap() != '.'
    {
        return (path.to_string(), "".to_string(), false);
    }

    let (prefix, path_major) = path.split_at(i - 2);

    if path_major.len() <= 2 || (path_major.chars().nth(2).unwrap() == '0' && path_major != ".v0") {
        return (path.to_string(), "".to_string(), false);
    }

    (prefix.to_string(), path_major.to_string(), true)
}

fn split_path_version(path: &str) -> (String, String, bool) {
    if path.starts_with("gopkg.in/") {
        return split_gopkg_in(path);
    }

    let mut i = path.len();
    let mut dot = false;

    while i > 0
        && (path.chars().nth(i - 1).unwrap().is_digit(10)
            || path.chars().nth(i - 1).unwrap() == '.')
    {
        if path.chars().nth(i - 1).unwrap() == '.' {
            dot = true;
        }
        i -= 1;
    }

    if i <= 1
        || i == path.len()
        || path.chars().nth(i - 1).unwrap() != 'v'
        || path.chars().nth(i - 2).unwrap() != '/'
    {
        return (path.to_string(), "".to_string(), true);
    }

    let (prefix, path_major) = path.split_at(i - 2);

    if dot
        || path_major.len() <= 2
        || path_major.chars().nth(2).unwrap() == '0'
        || path_major == "/v1"
    {
        return (path.to_string(), "".to_string(), false);
    }

    (prefix.to_string(), path_major.to_string(), true)
}

fn check_path_major(v: &str, path_major: &str) -> Result<(), InvalidVersionError> {
    if path_major.starts_with(".v") && path_major.ends_with("-unstable") {
        let path_major = path_major.trim_end_matches("-unstable");

        if v.starts_with("v0.0.0-") && path_major == ".v1" {
            // Allow old bug in pseudo-versions that generated v0.0.0- pseudoversion for gopkg .v1.
            // For example, gopkg.in/yaml.v2@v2.2.1's go.mod requires gopkg.in/check.v1 v0.0.0-20161208181325-20d25e280405.
            return Ok(());
        }

        let v_major = semserver::parse(v).unwrap().major;

        if path_major.is_empty() {
            if v_major == "0" || v_major == "1" || v.ends_with("+incompatible") {
                return Ok(());
            }
            return Err(InvalidVersionError {
                version: v.to_string(),
                pseudo: false,
                err: Box::try_from(format!(
                    "should be v0 or v1, not {}",
                    semserver::parse(v).unwrap().major
                ))
                .unwrap(),
            });
        } else if path_major.starts_with('/') || path_major.starts_with('.') {
            if v_major == semserver::parse(&path_major[1..]).unwrap().major {
                return Ok(());
            }
            return Err(InvalidVersionError {
                version: v.to_string(),
                pseudo: false,
                err: Box::try_from(format!(
                    "should be {}, not {}",
                    &path_major[1..],
                    semserver::parse(v).unwrap().major
                ))
                .unwrap(),
            });
        }
    }

    Ok(())
}

// Check checks that a given module path, version pair is valid.
// In addition to the path being a valid module path
// and the version being a valid semantic version,
// the two must correspond.
fn check(path: &str, version: &str) -> Result<(), Box<dyn Error>> {
    check_path(path)?;

    if !semserver::is_valid(version) {
        return Err(Box::new(InvalidVersionError {
            version: version.to_string(),
            pseudo: false,
            err: Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "not a semantic version",
            )),
        }));
    }

    let (_, path_major, _) = split_path_version(path);

    if let Err(e) = check_path_major(version, &path_major) {
        return Err(Box::new(ModuleError {
            path: path_major,
            version: Some(version.to_string()),
            err: Box::new(e),
        }));
    }

    Ok(())
}

// CheckPath checks that a given module path is valid.
// Specifically, it must contain at least one slash,
// must not start with a slash or dot,
// and must not contain any "bad" characters,
// meaning characters other than ASCII alphanumerics,
// directory separators, dots, underscores, and tildes.
fn check_path(path: &str) -> Result<(), Box<dyn Error>> {
    if !path.contains('/') {
        return Err(Box::new(InvalidPathError::new(
            "module",
            path.to_string(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "missing version",
            )),
        )));
    }
    if path.starts_with('/') || path.starts_with('.') {
        return Err(Box::new(InvalidPathError::new(
            "module",
            path.to_string(),
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "leading slash or dot in path",
            )),
        )));
    }
    for c in path.chars() {
        if !c.is_ascii_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '~' {
            return Err(Box::new(InvalidPathError::new(
                "module",
                path.to_string(),
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid character '{}'", c),
                )),
            )));
        }
    }
    Ok(())
}

// firstPathOK reports whether r can appear in the first element of a module path.
// The first element of the path must be an LDH domain name, at least for now.
// To avoid case ambiguity, the domain name must be entirely lower case.
fn first_path_ok(r: char) -> bool {
    r == '-' || r == '.' || ('0' <= r && r <= '9') || ('a' <= r && r <= 'z')
}

// modPathOK reports whether r can appear in a module path element.
// Paths can be ASCII letters, ASCII digits, and limited ASCII punctuation: - . _ and ~.
//
// This matches what "go get" has historically recognized in import paths,
// and avoids confusing sequences like '%20' or '+' that would change meaning
// if used in a URL.
//
// TODO(rsc): We would like to allow Unicode letters, but that requires additional
// care in the safe encoding (see "escaped paths" above).
fn mod_path_ok(r: char) -> bool {
    if r < '\u{80}' {
        return r == '-'
            || r == '.'
            || r == '_'
            || r == '~'
            || ('0' <= r && r <= '9')
            || ('A' <= r && r <= 'Z')
            || ('a' <= r && r <= 'z');
    }
    false
}

// importPathOK reports whether r can appear in a package import path element.
//
// Import paths are intermediate between module paths and file paths: we allow
// disallow characters that would be confusing or ambiguous as arguments to
// 'go get' (such as '@' and ' ' ), but allow certain characters that are
// otherwise-unambiguous on the command line and historically used for some
// binary names (such as '++' as a suffix for compiler binaries and wrappers).
fn import_path_ok(c: char) -> bool {
    return mod_path_ok(c) || c == '+';
}

// fileNameOK reports whether r can appear in a file name.
// For now we allow all Unicode letters but otherwise limit to pathOK plus a few more punctuation characters.
// If we expand the set of allowed characters here, we have to
// work harder at detecting potential case-folding and normalization collisions.
// See note about "escaped paths" above.
fn file_name_ok(r: char) -> bool {
    if r < '\u{80}' {
        // Entire set of ASCII punctuation, from which we remove characters:
        //     ! " # $ % & ' ( ) * + , - . / : ; < = > ? @ [ \ ] ^ _ ` { | } ~
        // We disallow some shell special characters: " ' * < > ? ` |
        // (Note that some of those are disallowed by the Windows file system as well.)
        // We also disallow path separators / : and \ (fileNameOK is only called on path element characters).
        // We allow spaces (U+0020) in file names.
        const ALLOWED: &str = "!#$%&()+,-.=@[]^_{}~ ";
        if ('0' <= r && r <= '9') || ('A' <= r && r <= 'Z') || ('a' <= r && r <= 'z') {
            return true;
        }
        return ALLOWED.contains(r);
    }
    // It may be OK to add more ASCII punctuation here, but only carefully.
    // For example Windows disallows < > \, and macOS disallows :, so we must not allow those.

    r.is_alphabetic()
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum PathKind {
    ModulePath,
    ImportPath,
    FilePath,
}

// badWindowsNames are the reserved file path elements on Windows.
// See https://docs.microsoft.com/en-us/windows/desktop/fileio/naming-a-file
const BAD_WINDOWS_NAMES: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

fn bad_windows_names() -> &'static [&'static str; 22] {
    &BAD_WINDOWS_NAMES
}

fn check_elem(elem: &str, kind: PathKind) -> Result<(), Box<dyn std::error::Error>> {
    if elem.is_empty() {
        return Err("empty path element".into());
    }
    if elem.chars().all(|c| c == '.') {
        return Err(format!("invalid path element {:?}", elem).into());
    }
    if elem.starts_with('.') && kind == PathKind::ModulePath {
        return Err("leading dot in path element".into());
    }
    if elem.ends_with('.') {
        return Err("trailing dot in path element".into());
    }
    for c in elem.chars() {
        let ok = match kind {
            PathKind::ModulePath => mod_path_ok(c),
            PathKind::ImportPath => import_path_ok(c),
            PathKind::FilePath => file_name_ok(c),
        };
        if !ok {
            return Err(format!("invalid char {:?}", c).into());
        }
    }

    // Windows disallows a bunch of path elements, sadly.
    // See https://docs.microsoft.com/en-us/windows/desktop/fileio/naming-a-file
    let short = if let Some(i) = elem.find('.') {
        &elem[..i]
    } else {
        elem
    };
    for bad in bad_windows_names() {
        if bad.eq_ignore_ascii_case(short) {
            return Err(format!(
                "{:?} disallowed as path element component on Windows",
                short
            )
            .into());
        }
    }

    if kind == PathKind::FilePath {
        // don't check for Windows short-names in file names. They're
        // only an issue for import paths.
        return Ok(());
    }

    // Reject path components that look like Windows short-names.
    // Those usually end in a tilde followed by one or more ASCII digits.
    if let Some(tilde) = short.rfind('~') {
        let suffix = &short[tilde + 1..];
        if suffix.chars().all(|c| c.is_ascii_digit()) {
            return Err("trailing tilde and digits in path element".into());
        }
    }

    Ok(())
}

fn _check_path(path: &str, kind: PathKind) -> Result<(), Box<dyn Error>> {
    if !path.is_char_boundary(path.len()) {
        return Err("invalid UTF-8".into());
    }
    if path.is_empty() {
        return Err("empty string".into());
    }
    if path.starts_with('-') && kind != PathKind::FilePath {
        return Err("leading dash".into());
    }
    if path.contains("//") {
        return Err("double slash".into());
    }
    if path.ends_with('/') {
        return Err("trailing slash".into());
    }
    let mut elem_start = 0;
    for (i, c) in path.char_indices() {
        if c == '/' {
            if let Err(err) = check_elem(&path[elem_start..i], kind) {
                return Err(err.into());
            }
            elem_start = i + c.len_utf8();
        }
    }
    if let Err(err) = check_elem(&path[elem_start..], kind) {
        return Err(err.into());
    }
    Ok(())
}

fn check_import_path(path: &str) -> Result<(), InvalidPathError> {
    if let Err(err) = _check_path(path, PathKind::ImportPath) {
        return Err(InvalidPathError {
            kind: "import",
            path: path.to_string(),
            err,
        });
    }
    Ok(())
}

// CheckFilePath checks that a slash-separated file path is valid.
// The definition of a valid file path is the same as the definition
// of a valid import path except that the set of allowed characters is larger:
// all Unicode letters, ASCII digits, the ASCII space character (U+0020),
// and the ASCII punctuation characters
// “!#$%&()+,-.=@[]^_{}~”.
// (The excluded punctuation characters, " * < > ? ` ' | / \ and :,
// have special meanings in certain shells or operating systems.)
//
// CheckFilePath may be less restrictive in the future, but see the
// top-level package documentation for additional information about
// subtleties of Unicode.
pub fn check_file_path(path: &str) -> Result<(), InvalidPathError> {
    Ok(_check_path(path, PathKind::FilePath).unwrap())
}

// MatchPathMajor reports whether the semantic version v
// matches the path major version pathMajor.
//
// MatchPathMajor returns true if and only if [CheckPathMajor] returns nil.
pub fn match_path_major(v: &str, path_major: &str) -> bool {
    check_path_major(v, path_major).is_ok()
}

// PathMajorPrefix returns the major-version tag prefix implied by pathMajor.
// An empty PathMajorPrefix allows either v0 or v1.
//
// Note that [MatchPathMajor] may accept some versions that do not actually begin
// with this prefix: namely, it accepts a 'v0.0.0-' prefix for a '.v1'
// pathMajor, even though that pathMajor implies 'v1' tagging.
fn path_major_prefix(path_major: &str) -> String {
    if path_major.is_empty() {
        return String::new();
    }

    if path_major.starts_with('/') || path_major.starts_with('.') {
        let mut m = &path_major[1..];

        if m.starts_with(".v") && m.ends_with("-unstable") {
            m = &m[..m.len() - "-unstable".len()];
        }

        if m != semserver::major(m) {
            panic!(
                "pathMajor suffix {} passed to PathMajorPrefix is not a valid major version",
                path_major
            );
        }

        return m.to_string();
    }

    panic!(
        "pathMajor suffix {} passed to PathMajorPrefix lacks separator",
        path_major
    );
}

// CanonicalVersion returns the canonical form of the version string v.
// It is the same as [semver.Canonical] except that it preserves the special build suffix "+incompatible".
pub fn canonical_version(v: &str) -> String {
    let mut cv = semserver::canonical(v);

    if semserver::build(v) == "+incompatible" {
        cv += "+incompatible";
    }

    cv
}

// Sort sorts the list by Path, breaking ties by comparing [Version] fields.
// The Version fields are interpreted as semantic versions (using [semver.Compare])
// optionally followed by a tie-breaking suffix introduced by a slash character,
// like in "v0.0.1/go.mod".
pub fn sort(list: &mut Vec<Version>) {
    list.sort_by(|mi, mj| {
        if mi.path != mj.path {
            return mi.path.cmp(&mj.path);
        }

        // To help go.sum formatting, allow version/file.
        // Compare semver prefix by semver rules,
        // file by string order.
        let (mut vi, mut fi) = (mi.version.clone(), String::new());
        let (mut vj, mut fj) = (mj.version.clone(), String::new());

        if let Some(k) = vi.find('/') {
            fi = vi.split_off(k);
        }

        if let Some(k) = vj.find('/') {
            fj = vj.split_off(k);
        }

        if vi != vj {
            vi.cmp(&vj)
        } else {
            fi.cmp(&fj)
        }
    });
}

fn escape_string(s: &str) -> Result<String, Box<dyn Error>> {
    let mut have_upper = false;
    for c in s.chars() {
        if c == '!' || c as u32 >= 0x80 {
            return Err("internal error: inconsistency in EscapePath".into());
        }
        if c.is_ascii_uppercase() {
            have_upper = true;
        }
    }

    if !have_upper {
        return Ok(s.to_owned());
    }

    let mut buf = String::new();
    for c in s.chars() {
        if c.is_ascii_uppercase() {
            buf.push('!');
            buf.push((c as u8 + b'a' - b'A') as char);
        } else {
            buf.push(c);
        }
    }

    Ok(buf)
}

// EscapePath returns the escaped form of the given module path.
// It fails if the module path is invalid.
pub fn escape_path(path: &str) -> Result<String, Box<dyn Error>> {
    // Assuming the CheckPath function validates the input path.
    check_path(path)?;

    let escaped = escape_string(path)?;
    Ok(escaped)
}

fn unescape_string(escaped: &str) -> (String, bool) {
    let mut buf = Vec::new();
    let mut bang = false;

    for r in escaped.chars() {
        if r >= '\u{80}' {
            return (String::new(), false);
        }

        if bang {
            bang = false;
            if r < 'a' || 'z' < r {
                return (String::new(), false);
            }
            buf.push((r as u8 + b'A' - b'a') as char);
            continue;
        }

        if r == '!' {
            bang = true;
            continue;
        }

        if 'A' <= r && r <= 'Z' {
            return (String::new(), false);
        }

        buf.push(r);
    }

    if bang {
        return (String::new(), false);
    }

    (buf.into_iter().collect(), true)
}

// UnescapePath returns the module path for the given escaped path.
// It fails if the escaped path is invalid or describes an invalid path.
fn unescape_path(escaped: &str) -> Result<String, Box<dyn Error>> {
    let (path, ok) = unescape_string(escaped);
    if !ok {
        return Err(format!("invalid escaped module path {}", escaped).into());
    }

    check_path(&path)?;

    Ok(path)
}

// UnescapeVersion returns the version string for the given escaped version.
// It fails if the escaped form is invalid or describes an invalid version.
// Versions are allowed to be in non-semver form but must be valid file names
// and not contain exclamation marks.
pub fn unescape_version(escaped: &str) -> Result<String, String> {
    let (v, ok) = unescape_string(escaped);
    if !ok {
        return Err(format!("invalid escaped version {}", escaped));
    }

    if let Err(err) = check_elem(&v, PathKind::FilePath) {
        return Err(format!("invalid escaped version {}: {}", v, err));
    }

    Ok(v)
}

// MatchPrefixPatterns reports whether any path prefix of target matches one of
// the glob patterns (as defined by [path.Match]) in the comma-separated globs
// list. This implements the algorithm used when matching a module path to the
// GOPRIVATE environment variable, as described by 'go help module-private'.
//
// It ignores any empty or malformed patterns in the list.
// Trailing slashes on patterns are ignored.
fn match_prefix_patterns(globs: &str, target: &str) -> bool {
    for glob in globs.split(',') {
        let glob = glob.trim_end_matches('/');
        if glob.is_empty() {
            continue;
        }

        // A glob with N+1 path elements (N slashes) needs to be matched
        // against the first N+1 path elements of target,
        // which end just before the N+1'th slash.
        let mut n = glob.chars().filter(|&c| c == '/').count();
        let mut prefix = target;
        // Walk target, counting slashes, truncating at the N+1'th slash.
        for (i, c) in target.chars().enumerate() {
            if c == '/' {
                if n == 0 {
                    prefix = &target[..i];
                    break;
                }
                n -= 1;
            }
        }
        if n > 0 {
            // Not enough prefix elements.
            continue;
        }
        if let Ok(matched) = Pattern::new(glob).map(|p| p.matches(prefix)) {
            if matched {
                return true;
            }
        }
    }
    false
}

lazy_static::lazy_static! {
    static ref PSEUDO_VERSION_RE: &'static regex::Regex = regex!(r#"^v[0-9]+\.(0\.0-|\d+\.\d+-([^+]*\.)?0\.)\d{14}-[A-Za-z0-9]+(\+[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$"#);
}

const PSEUDO_VERSION_TIMESTAMP_FORMAT: &str = "%Y%m%d%H%M%S";
fn pseudo_version(major: &str, older: &str, t: DateTime<Utc>, rev: &str) -> String {
    let major = if major.is_empty() { "v0" } else { major };
    let segment = format!("{}-{}", t.format(PSEUDO_VERSION_TIMESTAMP_FORMAT), rev);
    let build = semserver::build(older);
    let older = semserver::canonical(older);

    if older.is_empty() {
        return format!("{}.0.0-{}", major, segment); // form (1)
    }

    if !semserver::prerelease(&older).is_empty() {
        return format!("{}.0.{}{}", older, segment, build); // form (4), (5)
    }

    // Form (2), (3).
    // Extract patch from vMAJOR.MINOR.PATCH
    let i = older.rfind('.').map_or(0, |index| index + 1);
    let (v, patch) = (
        older.as_str().get(..i).unwrap_or_default(),
        older.as_str().get(i..).unwrap_or_default(),
    );

    //v1.2.3-pre.0.20060102150405-hash
    //v1.2.3-pre-0.20060102150405-hash
    //panic!("{}", format!("{}-{}", v, inc_decimal(patch)));
    //v + incDecimal(patch) + "-0." + segment + build
    // Reassemble.
    return format!("{}{}-0.{}{}", v, inc_decimal(patch), segment, build);
}

fn inc_decimal(decimal: &str) -> String {
    let d = Decimal::from_str(decimal).unwrap();
    d.add(Decimal::from_str("1").unwrap()).to_string()
}

// ZeroPseudoVersion returns a pseudo-version with a zero timestamp and
// revision, which may be used as a placeholder.
pub fn zero_pseudo_version(major: &str) -> String {
    pseudo_version(major, "", Utc.timestamp_opt(0, 0).unwrap(), "000000000000")
}

fn dec_decimal(decimal: &str) -> String {
    let mut digits: Vec<u8> = decimal.bytes().collect();
    let mut i = digits.len();

    while i > 0 && digits[i - 1] == b'0' {
        i -= 1;
        digits[i] = b'9';
    }

    if i == 0 {
        // decimal is all zeros
        "".to_string()
    } else {
        i -= 1;
        if i == 0 && digits[i] == b'1' && digits.len() > 1 {
            digits = digits[1..].to_vec();
        } else {
            digits[i] -= 1;
        }
        String::from_utf8(digits).unwrap()
    }
}

// IsPseudoVersion reports whether v is a pseudo-version.
pub fn is_pseudo_version(v: &str) -> bool {
    //return strings.Count(v, "-") >= 2 && semver.IsValid(v) && pseudoVersionRE.MatchString(v)
    v.matches('-').count() >= 2 && semserver::parse(v).is_ok() && PSEUDO_VERSION_RE.is_match(v)
}

// IsZeroPseudoVersion returns whether v is a pseudo-version with a zero base,
// timestamp, and revision, as returned by [ZeroPseudoVersion].
fn is_zero_pseudo_version(v: &str) -> bool {
    v == zero_pseudo_version(
        &semserver::parse(v)
            .map(|ver| ver.major.to_string())
            .unwrap_or_default(),
    )
}

fn pseudo_version_time(v: &str) -> Result<DateTime<Utc>, InvalidVersionError> {
    let (_, timestamp, _, _) = parse_pseudo_version(v)?;

    NaiveDateTime::parse_from_str(&timestamp, PSEUDO_VERSION_TIMESTAMP_FORMAT)
        .map(|ndt| ndt.and_utc())
        .map_err(|err| InvalidVersionError {
            version: v.to_string(),
            pseudo: true,
            err: Box::new(err),
        })
}

#[derive(Debug)]
struct SyntaxErr {
    message: String,
}

impl fmt::Display for SyntaxErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for SyntaxErr {}

fn parse_pseudo_version(
    mut v: &str,
) -> Result<(String, String, String, String), InvalidVersionError> {
    if !is_pseudo_version(v) {
        let my_error: Box<dyn Error> = Box::new(SyntaxErr {
            message: "syntax error".to_string(),
        });

        return Err(InvalidVersionError {
            version: v.to_string(),
            err: my_error,
            pseudo: true,
        });
    }

    let build = semserver::build(v);
    v = v.strip_suffix(&build).unwrap();
    let j = v
        .char_indices()
        .rev()
        .find_map(|(i, c)| if c == '-' { Some(i) } else { None })
        .unwrap_or_default();

    let (vv, rev) = (&v[..j], &v[j + 1..]);

    let i = vv
        .char_indices()
        .rev()
        .find_map(|(i, c)| if c == '-' { Some(i) } else { None })
        .unwrap_or_default();

    let j = vv
        .char_indices()
        .rev()
        .find_map(|(i, c)| if c == '.' { Some(i) } else { None })
        .unwrap_or_default();

    let ind = usize::max(j, i);
    let (base, timestamp) = (vv.get(..ind).unwrap(), vv.get(ind + 1..).unwrap());

    Ok((
        base.to_string(),
        timestamp.to_string(),
        rev.to_string(),
        build.to_string(),
    ))
}

fn pseudo_version_rev(v: &str) -> Result<String, InvalidVersionError> {
    parse_pseudo_version(v).map(|(_, _, rev, _)| rev)
}

fn pseudo_version_base(v: &str) -> Result<String, InvalidVersionError> {
    let (base, _, _, build) = parse_pseudo_version(v)?;
    let pre = semserver::prerelease(&base);

    match pre.as_str() {
        "" => {
            if !build.is_empty() {
                return Err(InvalidVersionError {
                    version: v.to_string(),
                    pseudo: true,
                    err: Box::new(fmt::Error),
                });
            }
            Ok(String::new())
        }
        "-0" => {
            let base = base.strip_suffix(&pre).unwrap_or(&pre);

            let i = base.rfind('.');
            if let Some(i) = i {
                let patch = dec_decimal(&base[i + 1..]);
                if !patch.is_empty() {
                    return Ok(format!("{}{}{}", &base[..i + 1], patch, build));
                }
                return Err(InvalidVersionError {
                    version: v.to_string(),
                    pseudo: true,
                    err: Box::new(fmt::Error),
                });
            } else {
                panic!(
                    "base from parsePseudoVersion missing patch number: {}",
                    base
                );
            }
        }
        _ => {
            if !base.ends_with(".0") {
                panic!(
                    "base from parsePseudoVersion missing \".0\" before date: {}",
                    base
                );
            }
            Ok(base.trim_end_matches(".0").to_string() + &build)
        }
    }
}

#[cfg(test)]
mod test {

    use super::*;
    static PSEUDO_TESTS: &[(&str, &str, &str)] = &[
        ("", "", "v0.0.0-20060102150405-hash"),
        ("v0", "", "v0.0.0-20060102150405-hash"),
        ("v1", "", "v1.0.0-20060102150405-hash"),
        ("v2", "", "v2.0.0-20060102150405-hash"),
        ("unused", "v0.0.0", "v0.0.1-0.20060102150405-hash"),
        ("unused", "v1.2.3", "v1.2.4-0.20060102150405-hash"),
        (
            "unused",
            "v1.2.99999999999999999",
            "v1.2.100000000000000000-0.20060102150405-hash",
        ),
        ("unused", "v1.2.3-pre", "v1.2.3-pre.0.20060102150405-hash"),
        ("unused", "v1.3.0-pre", "v1.3.0-pre.0.20060102150405-hash"),
        ("unused", "v0.0.0--", "v0.0.0--.0.20060102150405-hash"),
        (
            "unused",
            "v1.0.0+metadata",
            "v1.0.1-0.20060102150405-hash+metadata",
        ),
        (
            "unused",
            "v2.0.0+incompatible",
            "v2.0.1-0.20060102150405-hash+incompatible",
        ),
        (
            "unused",
            "v2.3.0-pre+incompatible",
            "v2.3.0-pre.0.20060102150405-hash+incompatible",
        ),
    ];

    use chrono::{DateTime, Utc};

    fn pseudo_time() -> DateTime<Utc> {
        Utc.ymd(2006, 1, 2).and_hms(15, 4, 5)
    }

    #[test]
    fn test_pseudo_version() {
        for tt in PSEUDO_TESTS {
            let v = pseudo_version(tt.0, tt.1, pseudo_time(), "hash");
            if v != tt.2 {
                eprintln!(
                    "pseudo_version({}, {}, ...), want {} got {}",
                    tt.0, tt.1, tt.2, v
                );
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_is_pseudo_version() {
        for tt in PSEUDO_TESTS {
            if !is_pseudo_version(tt.2) {
                eprintln!("is_pseudo_version({}) = false, want true", tt.2);
                panic!("Test failed");
            }
            if is_pseudo_version(tt.1) {
                eprintln!("is_pseudo_version({}) = true, want false", tt.1);
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_pseudo_version_time() {
        for tt in PSEUDO_TESTS {
            match pseudo_version_time(tt.2) {
                Ok(tm) => {
                    let want = pseudo_time();
                    let got = tm;

                    assert_eq!(want, got, "arg: {} wanted: {} got: {}", tt.2, want, got,);
                }
                Err(err) => {
                    panic!("err: {}", err);
                }
            }

            if let Ok(tm) = pseudo_version_time(tt.1) {
                assert_eq!(tm, Utc.timestamp_opt(0, 0).unwrap(), "arg: {}", tt.1,);
            }
        }
    }

    #[test]
    fn test_invalid_pseudo_version_time() {
        const V: &str = "---";
        if let Ok(_) = pseudo_version_time(V) {
            eprintln!("pseudo_version_time({}) = Ok, want Error", V);
            panic!("Test failed");
        }
    }

    #[test]
    fn test_pseudo_version_rev() {
        for tt in PSEUDO_TESTS {
            match pseudo_version_rev(tt.2) {
                Ok(rev) => {
                    if rev != "hash" {
                        eprintln!("pseudo_version_rev({}) = {}, want hash", tt.2, rev);
                        panic!("Test failed");
                    }
                }
                Err(err) => {
                    eprintln!("pseudo_version_rev({}) = Error, want hash", tt.2);
                    panic!("Test failed");
                }
            }

            if let Ok(rev) = pseudo_version_rev(tt.1) {
                if rev != "" {
                    eprintln!("pseudo_version_rev({}) = {}, want empty", tt.1, rev);
                    panic!("Test failed");
                }
            }
        }
    }

    #[test]
    fn test_pseudo_version_base() {
        for tt in PSEUDO_TESTS {
            match pseudo_version_base(tt.2) {
                Ok(base) => {
                    if base != tt.1 {
                        eprintln!("pseudo_version_base({}) = {}, want {}", tt.2, base, tt.1);
                        panic!("Test failed");
                    }
                }
                Err(err) => {
                    eprintln!("pseudo_version_base({}) = Error: {}", tt.2, err);
                    panic!("Test failed");
                }
            }
        }
    }

    #[test]
    fn test_invalid_pseudo_version_base() {
        for &input in &[
            "v0.0.0",
            "v0.0.0-",                                 // malformed: empty prerelease
            "v0.0.0-0.20060102150405-hash",            // Z+1 == 0
            "v0.1.0-0.20060102150405-hash",            // Z+1 == 0
            "v1.0.0-0.20060102150405-hash",            // Z+1 == 0
            "v0.0.0-20060102150405-hash+incompatible", // "+incompatible without base version
            "v0.0.0-20060102150405-hash+metadata",     // other metadata without base version
        ] {
            if let Ok(s) = pseudo_version_base(input) {
                assert!(s.is_empty());
            }
        }
    }

    #[test]
    fn test_inc_decimal() {
        let cases = vec![
            ("0", "1"),
            ("1", "2"),
            ("99", "100"),
            ("100", "101"),
            ("101", "102"),
        ];

        for (input, expected) in cases {
            let result = inc_decimal(input);
            if result != expected {
                eprintln!("inc_decimal({}) = {}, want = {}", input, result, expected);
                panic!("Test failed");
            }
        }
    }

    #[test]
    fn test_dec_decimal() {
        let cases = vec![
            ("", ""),
            ("0", ""),
            ("00", ""),
            ("1", "0"),
            ("2", "1"),
            ("99", "98"),
            ("100", "99"),
            ("101", "100"),
        ];

        for (input, expected) in cases {
            let result = dec_decimal(input);
            if &result != expected {
                eprintln!(
                    "dec_decimal({:#?}) = {:#?}, want = {:#?}",
                    input,
                    result,
                    expected.to_string()
                );
                panic!("Test failed");
            }
        }
    }
}
