// Copyright 2018 The Go Authors. All rights reserved.
// Use of this source code is governed by a BSD-style
// license that can be found in the LICENSE file.

use std::cmp::Ordering;

// Parsed represents the parsed form of a semantic version string.
#[derive(Debug)]
struct Parsed {
    major: String,
    minor: String,
    patch: String,
    short: String,
    prerelease: String,
    build: String,
}

// IsValid reports whether v is a valid semantic version string.
fn is_valid(v: &str) -> bool {
    parse(v).is_ok()
}

// Canonical returns the canonical formatting of the semantic version v.
// It fills in any missing .MINOR or .PATCH and discards build metadata.
// Two semantic versions compare equal only if their canonical formattings
// are identical strings.
// The canonical invalid semantic version is the empty string.
fn canonical(v: &str) -> String {
    match parse(v) {
        Ok(p) => {
            if !p.build.is_empty() {
                return v[..v.len() - p.build.len()].to_string();
            }
            if !p.short.is_empty() {
                return v.to_string() + &p.short;
            }
            v.to_string()
        }
        Err(e) => {
            println!("{}", e);
            String::new()
        }
    }
}

// Major returns the major version prefix of the semantic version v.
fn major(v: &str) -> String {
    if let Ok(pv) = parse(v) {
        return v[..1 + pv.major.len()].to_string();
    }
    String::new()
}

// MajorMinor returns the major.minor version prefix of the semantic version v.
fn major_minor(v: &str) -> String {
    if let Ok(pv) = parse(v) {
        let chars: Vec<char> = v.chars().collect();
        let i = 1 + pv.major.len();
        let j = i + 1 + pv.minor.len();

        if j <= chars.len() && chars[i] == '.' && v[i + 1..j] == pv.minor {
            return v[..j].to_string();
        }
        return v[..i].to_string() + "." + &pv.minor;
    }
    String::new()
}

// Prerelease returns the prerelease suffix of the semantic version v.
fn prerelease(v: &str) -> String {
    if let Ok(pv) = parse(v) {
        return pv.prerelease.to_string();
    }
    String::new()
}

// Build returns the build suffix of the semantic version v.
fn build(v: &str) -> String {
    if let Ok(pv) = parse(v) {
        return pv.build.to_string();
    }
    String::new()
}

// Compare returns an integer comparing two versions according to
// semantic version precedence.
// The result will be 0 if v == w, -1 if v < w, or +1 if v > w.
//
// An invalid semantic version string is considered less than a valid one.
// All invalid semantic version strings compare equal to each other.
fn compare(v: &str, w: &str) -> Ordering {
    match (parse(v), parse(w)) {
        (Ok(pv), Ok(pw)) => {
            let major_cmp = pv.major.cmp(&pw.major);
            if major_cmp != Ordering::Equal {
                return major_cmp;
            }

            let minor_cmp = pv.minor.cmp(&pw.minor);
            if minor_cmp != Ordering::Equal {
                return minor_cmp;
            }

            let patch_cmp = pv.patch.cmp(&pw.patch);

            if patch_cmp != Ordering::Equal {
                return patch_cmp;
            }

            return compare_prerelease(&pv.prerelease, &pw.prerelease);
        }
        (Err(_), Err(_)) => Ordering::Equal,
        (Ok(_), Err(_)) => Ordering::Greater,
        (Err(_), Ok(_)) => Ordering::Less,
    }
}

// Max canonicalizes its arguments and then returns the version string
// that compares greater.
//
// Deprecated: use Compare instead. In most cases, returning a canonicalized
// version is not expected or desired.
#[deprecated]
fn max(v: &str, w: &str) -> String {
    let v = canonical(v);

    let w = canonical(w);

    if v.is_empty() {
        return w;
    }

    if w.is_empty() {
        return v;
    }

    if compare(&v, &w) > Ordering::Equal {
        return v;
    }
    w
}

// Sort sorts a list of semantic version strings using ByVersion.
fn sort(list: &mut Vec<&str>) {
    list.sort_by(|a, b| compare(a, b));
}

fn parse(v: &str) -> Result<Parsed, &'static str> {
    if v.is_empty() || !v.starts_with('v') {
        return Err("Invalid version string");
    }

    let mut rest = &v[1..];
    let mut parsed = Parsed {
        major: "".to_string(),
        minor: "".to_string(),
        patch: "".to_string(),
        short: "".to_string(),
        prerelease: "".to_string(),
        build: "".to_string(),
    };

    parsed.major = parse_int(&mut rest).expect("major");
    //panic!("rest: {}", rest);
    if rest.is_empty() {
        parsed.minor = "0".to_string();
        parsed.patch = "0".to_string();
        parsed.short = ".0.0".to_string();

        return Ok(parsed);
    }

    if !rest.starts_with('.') {
        return Err("Invalid version string");
    }
    rest = &rest[1..];

    parsed.minor = parse_int(&mut rest).expect("minor");
    if rest.is_empty() {
        parsed.patch = "0".to_string();
        parsed.short = ".0".to_string();
        return Ok(parsed);
    }

    if !rest.starts_with('.') {
        return Err("Invalid version string");
    }
    rest = &rest[1..];

    parsed.patch = parse_int(&mut rest).expect("patch");
    if rest.is_empty() {
        return Ok(parsed);
    }

    if rest.starts_with('-') {
        parsed.prerelease = parse_prerelease(&mut rest)?;
    }

    if !rest.is_empty() && rest.starts_with('+') {
        parsed.build = parse_build(&mut rest)?;
    }

    Ok(parsed)
}

fn parse_segment(v: &mut &str) -> Result<String, &'static str> {
    if v.is_empty() || v.chars().nth(0) != Some('v') {
        return Err("1");
    }
    let mut i = 1;
    let mut start = 1;

    while let Some(c) = v.chars().nth(i) {
        if c == '+' {
            break;
        } else if !is_ident_char(c) && c != '.' {
            return Err("2");
        } else if c == '.' {
            if start == i || is_bad_num(&v[start..i]) {
                return Err("3");
            }
            start = i + 1;
        }
        i += 1;
    }

    if start == i || is_bad_num(&v[start..i]) {
        return Err("4");
    }

    let r = v[i..].to_string();
    *v = &v[..i];
    Ok(r)
}

fn parse_prerelease(rest: &mut &str) -> Result<String, &'static str> {
    let chars: Vec<char> = rest.chars().collect();

    if rest.is_empty() || chars[0] != '-' {
        return Err("Invalid version string");
    }

    let mut i = 1;
    let mut start = 1;

    while i < chars.len() && chars[i] != '+' {
        if !is_ident_char(chars[i]) && chars[i] != '.' {
            return Err("Invalid character");
        }
        if chars[i] == '.' {
            if start == i || is_bad_num(rest.get(start..i).unwrap()) {
                return Err("Invalid character");
            }
            start = i + 1;
        }
        i += 1;
    }
    if start == i || is_bad_num(rest.get(start..i).unwrap()) {
        return Err("Bad num");
    }
    let segment = rest[..i].to_string();
    *rest = &rest[i..];
    Ok(segment)
}

fn parse_int(v: &mut &str) -> Result<String, ()> {
    if v.is_empty() {
        return Err(());
    }

    let chars: Vec<char> = v.chars().collect();

    if chars[0] < '0' || '9' < chars[0] {
        return Err(());
    }

    let mut i = 1;
    while i < chars.len() && '0' <= chars[i] && chars[i] <= '9' {
        i += 1;
    }

    if chars[0] == '0' && i != 1 {
        return Err(());
    }

    let r = v[..i].to_string();
    *v = &v[i..];

    Ok(r)
}

fn parse_build(rest: &mut &str) -> Result<String, &'static str> {
    let chars: Vec<char> = rest.chars().collect();

    if rest.is_empty() || chars[0] != '+' {
        return Err("Invalid build string");
    }

    let mut i = 1;
    let mut start = 1;

    while i < chars.len() {
        if !is_ident_char(chars[i]) && chars[i] != '.' {
            return Err("Invalid char");
        }
        if chars[i] == '.' {
            if start == i {
                return Err("Invalid char");
            }
            start = i + 1
        }
        i += 1;
    }
    if start == i {
        return Err("Invalid char");
    }
    let segment = rest[..i].to_string();
    *rest = &rest[i..];
    Ok(segment)
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

fn is_bad_num(v: &str) -> bool {
    let mut i = 0;
    while let Some(c) = v.chars().nth(i) {
        if !c.is_digit(10) {
            break;
        }
        i += 1;
    }
    i == v.len() && i > 1 && v.chars().next().unwrap() == '0'
}

fn is_num(v: &str) -> bool {
    let mut i = 0;
    while let Some(c) = v.chars().nth(i) {
        if !c.is_digit(10) {
            break;
        }
        i += 1;
    }
    i == v.len()
}

fn compare_int(x: &str, y: &str) -> Ordering {
    x.cmp(y)
}

fn compare_prerelease(mut x: &str, mut y: &str) -> Ordering {
    if x == y {
        return Ordering::Equal;
    }
    if x.is_empty() {
        return Ordering::Greater;
    }
    if y.is_empty() {
        return Ordering::Less;
    }

    while x != "" && y != "" {
        x = x.get(1..).unwrap(); // skip - or .
        y = y.get(1..).unwrap(); // skip - or .

        let dx = next_ident(&mut x);
        let dy = next_ident(&mut y);

        if dx != dy {
            let ix = is_num(&dx);
            let iy = is_num(&dy);

            if ix != iy {
                return if ix {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
            if ix {
                if dx.len() < dy.len() {
                    return Ordering::Less;
                }
                if dx.len() > dy.len() {
                    return Ordering::Greater;
                }
            }
            return if dx < dy {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
    }
    return if x == "" {
        Ordering::Less
    } else {
        Ordering::Greater
    };
}

fn next_ident(s: &mut &str) -> String {
    let i = s.find('.').unwrap_or(s.len());
    let (dx, rest) = s.split_at(i);
    let r = dx.to_string();
    *s = rest;
    r
}

#[cfg(test)]
mod test {
    use super::*;

    use rand::seq::SliceRandom;
    use std::cmp::Ordering;

    #[derive(Debug, PartialEq, Eq)]
    struct Test {
        input: &'static str,
        output: &'static str,
    }

    const TESTS: &[Test] = &[
        Test {
            input: "bad",
            output: "",
        },
        Test {
            input: "v1-alpha.beta.gamma",
            output: "",
        },
        Test {
            input: "v1-pre",
            output: "",
        },
        Test {
            input: "v1+meta",
            output: "",
        },
        Test {
            input: "v1-pre+meta",
            output: "",
        },
        Test {
            input: "v1.2-pre",
            output: "",
        },
        Test {
            input: "v1.2+meta",
            output: "",
        },
        Test {
            input: "v1.2-pre+meta",
            output: "",
        },
        Test {
            input: "v1.0.0-alpha",
            output: "v1.0.0-alpha",
        },
        Test {
            input: "v1.0.0-alpha.1",
            output: "v1.0.0-alpha.1",
        },
        Test {
            input: "v1.0.0-alpha.beta",
            output: "v1.0.0-alpha.beta",
        },
        Test {
            input: "v1.0.0-beta",
            output: "v1.0.0-beta",
        },
        Test {
            input: "v1.0.0-beta.2",
            output: "v1.0.0-beta.2",
        },
        Test {
            input: "v1.0.0-beta.11",
            output: "v1.0.0-beta.11",
        },
        Test {
            input: "v1.0.0-rc.1",
            output: "v1.0.0-rc.1",
        },
        Test {
            input: "v1",
            output: "v1.0.0",
        },
        Test {
            input: "v1.0",
            output: "v1.0.0",
        },
        Test {
            input: "v1.0.0",
            output: "v1.0.0",
        },
        Test {
            input: "v1.2",
            output: "v1.2.0",
        },
        Test {
            input: "v1.2.0",
            output: "v1.2.0",
        },
        Test {
            input: "v1.2.3-456",
            output: "v1.2.3-456",
        },
        Test {
            input: "v1.2.3-456.789",
            output: "v1.2.3-456.789",
        },
        Test {
            input: "v1.2.3-456-789",
            output: "v1.2.3-456-789",
        },
        Test {
            input: "v1.2.3-456a",
            output: "v1.2.3-456a",
        },
        Test {
            input: "v1.2.3-pre",
            output: "v1.2.3-pre",
        },
        Test {
            input: "v1.2.3-pre+meta",
            output: "v1.2.3-pre",
        },
        Test {
            input: "v1.2.3-pre.1",
            output: "v1.2.3-pre.1",
        },
        Test {
            input: "v1.2.3-zzz",
            output: "v1.2.3-zzz",
        },
        Test {
            input: "v1.2.3",
            output: "v1.2.3",
        },
        Test {
            input: "v1.2.3+meta",
            output: "v1.2.3",
        },
        Test {
            input: "v1.2.3+meta-pre",
            output: "v1.2.3",
        },
        Test {
            input: "v1.2.3+meta-pre.sha.256a",
            output: "v1.2.3",
        },
    ];

    #[test]
    fn test_is_valid() {
        for tt in TESTS {
            let is_valid = is_valid(tt.input);
            assert_eq!(is_valid, tt.output != "");
        }
    }

    #[test]
    fn test_canonical() {
        for tt in TESTS {
            let canonical = canonical(tt.input);
            assert_eq!(canonical, tt.output);
        }
    }

    #[test]
    fn test_major() {
        for tt in TESTS {
            let major = major(tt.input);
            let want = if let Some(i) = tt.output.find('.') {
                &tt.output[..i]
            } else {
                ""
            };
            assert_eq!(major, want);
        }
    }

    #[test]
    fn test_major_minor() {
        for tt in TESTS {
            let major_minor = major_minor(tt.input);
            let mut want = String::new();

            if !tt.output.is_empty() {
                want = tt.input.to_string();

                if let Some(i) = want.find('+') {
                    want.truncate(i);
                }
                if let Some(i) = want.find('-') {
                    want.truncate(i);
                }
                match want.matches('.').count() {
                    0 => want.push_str(".0"),
                    2 => want.truncate(want.rfind('.').unwrap()),
                    _ => {}
                }
            }

            assert_eq!(major_minor, want);
        }
    }

    #[test]
    fn test_prerelease() {
        for tt in TESTS {
            let prerelease = prerelease(tt.input);
            let mut want = String::new();
            if let Some(i) = tt.output.find('-') {
                want = tt.output[i..].to_string();
            }
            assert_eq!(prerelease, want);
        }
    }

    #[test]
    fn test_build() {
        for tt in TESTS {
            let build = build(tt.input);
            let mut want = String::new();

            if !tt.output.is_empty() {
                if let Some(i) = tt.input.find('+') {
                    want.push_str(&tt.input[i..]);
                }
            }
            assert_eq!(build, want, "{}", tt.input);
        }
    }

    #[test]
    fn test_compare() {
        for (i, ti) in TESTS.iter().enumerate() {
            for (j, tj) in TESTS.iter().enumerate() {
                let cmp = compare(ti.input, tj.input);
                let want = if ti.output == tj.output {
                    Ordering::Equal
                } else if i < j {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
                assert_eq!(cmp, want, "{:#?}  {:#?}", ti.input, tj.input);
            }
        }
    }

    #[test]
    fn test_sort() {
        let mut versions: Vec<_> = TESTS.iter().map(|test| test.input).collect();
        let mut rng = rand::thread_rng();
        versions.shuffle(&mut rng);
        sort(&mut versions);
        assert!(versions
            .windows(2)
            .all(|w| compare(w[0], w[1]) <= Ordering::Equal));
    }

    #[test]
    fn test_max() {
        for (i, ti) in TESTS.iter().enumerate() {
            for (j, tj) in TESTS.iter().enumerate() {
                let max = max(ti.input, tj.input);
                let want = if i < j {
                    canonical(tj.input)
                } else {
                    canonical(ti.input)
                };
                assert_eq!(
                    max, want,
                    "max: {} =  {:#?} -- {:#?} -- want: {:#?} i: {} j: {}",
                    max, ti.input, tj.input, want, i, j
                );
            }
        }
    }

    const V1: &str = "v1.0.0+metadata-dash";
    const V2: &str = "v1.0.0+metadata-dash1";

    // #[test]
    // fn bench_compare(b: &mut test::Bencher) {
    //     b.iter(|| {
    //         if compare(V1, V2) != 0 {
    //             panic!("bad compare");
    //         }
    //     });
    // }
}
