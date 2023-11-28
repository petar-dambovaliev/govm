use crate::gomod::semserver;
use glob::Pattern;
use std::error::Error;

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
