//! A player ID's backup file (ducy-web #152, #436, #460): what it says
//! without the password, the passwords new backups take, and a club's backup
//! (the same file with the club it brings back).
//!
//! The private key in it is locked and unlocked in the browser's WebCrypto
//! (PBKDF2-SHA-256 then AES-256-GCM, with the header bound in as associated
//! data); this only reads and checks the file around it.
//!
//! - Version 1: `{type: "ducy-identity", v: 1, id, alg, seq, key}`, the
//!   private key (a JWK) in the clear. Older backups, and an always-on club's
//!   own identity on the relay.
//! - Version 2: `{type, v: 2, id, alg, seq, lock: {kdf: "PBKDF2-SHA256",
//!   iterations, salt, iv, ct}}`.
//! - A club's backup adds `club: {code, name}`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The shortest password a new backup takes.
pub const MIN_NEW_PASSWORD: usize = 12;
/// The shortest an older backup may have been locked with.
pub const MIN_PASSWORD: usize = 8;
/// How many PBKDF2 rounds a new backup is locked with.
pub const PBKDF2_ITERATIONS: u64 = 600_000;

const NOT_A_FILE: &str = "That isn’t a ducy identity file.";

/// Passwords too common to lock a backup with, as [`core`] makes them: digits
/// and symbols on the end, or in place of letters, don't help them.
const COMMON_PASSWORDS: &[&str] = &[
    "password",
    "passwort",
    "motdepasse",
    "contraseña",
    "secret",
    "letmein",
    "welcome",
    "iloveyou",
    "admin",
    "administrator",
    "login",
    "qwerty",
    "qwertz",
    "azerty",
    "asdf",
    "zxcv",
    "abc",
    "abcd",
    "monkey",
    "dragon",
    "football",
    "baseball",
    "basketball",
    "soccer",
    "hockey",
    "sunshine",
    "princess",
    "trustno",
    "starwars",
    "whatever",
    "superman",
    "batman",
    "master",
    "shadow",
    "freedom",
    "changeme",
    "default",
    "computer",
    "internet",
    "hello",
    "hellohello",
    "loveyou",
    "iloveyouiloveyou",
    "mypassword",
    "yourpassword",
    "thepassword",
    "newpassword",
    "backup",
    "mybackup",
    "correcthorsebatterystaple",
    "poker",
    "pokerface",
    "pokerclub",
    "mypokerclub",
    "holdem",
    "texasholdem",
    "allin",
    "acesfull",
    "royalflush",
    "ducy",
    "ducycards",
    "ducypoker",
    "ducyclub",
];

/// Runs along the digits, the alphabet and the keyboard's rows.
const RUNS: &[&str] = &[
    "01234567890123456789",
    "98765432109876543210",
    "abcdefghijklmnopqrstuvwxyz",
    "zyxwvutsrqponmlkjihgfedcba",
    "qwertyuiopasdfghjklzxcvbnm",
    "1qaz2wsx3edc4rfv5tgb6yhn7ujm",
];

/// Lower case, letters and digits only.
fn plain(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

/// As [`plain`], without what's after the last letter, and with look-alikes
/// (P@ssw0rd) as the letters they stand for.
fn core(s: &str) -> String {
    let lower = s.to_lowercase();
    let trimmed = lower.trim_end_matches(|c: char| !c.is_alphabetic());
    let swapped: String = trimmed
        .chars()
        .map(|c| match c {
            '@' | '4' => 'a',
            '0' => 'o',
            '1' | '!' => 'i',
            '3' => 'e',
            '5' | '$' => 's',
            '7' => 't',
            c => c,
        })
        .collect();
    plain(&swapped)
}

/// Whether `s` is a unit of 1 to 4 characters repeated at least twice, with
/// at most 3 more after it.
fn repeated(s: &[char]) -> bool {
    (1..=4).any(|u| {
        if s.len() < 2 * u {
            return false;
        }
        let unit = &s[..u];
        let reps = s.chunks(u).take_while(|c| *c == unit).count();
        reps >= 2 && s.len() - reps * u <= 3
    })
}

/// Why `password` won't do for a new backup, in one line, or "" if it will:
/// too short, very common, just the club's `name`, or a pattern.
pub fn password_problem(password: &str, name: &str) -> String {
    if password.chars().count() < MIN_NEW_PASSWORD {
        return format!("Use a password of at least {MIN_NEW_PASSWORD} characters.");
    }
    let (p, k, club) = (plain(password), core(password), plain(name));
    let trailing_digits = p.trim_end_matches(|c: char| c.is_ascii_digit());
    if !club.is_empty()
        && (p == club || k == core(name) || trailing_digits == club || p == format!("{club}{club}"))
    {
        return "Don’t use the club’s name as its password.".into();
    }
    if COMMON_PASSWORDS.contains(&p.as_str()) || COMMON_PASSWORDS.contains(&k.as_str()) {
        return "That password is too common.".into();
    }
    let lower: Vec<char> = password.to_lowercase().chars().collect();
    let mut distinct = lower.clone();
    distinct.sort_unstable();
    distinct.dedup();
    if repeated(&lower) || distinct.len() <= 2 {
        return "That password is just repeated characters.".into();
    }
    if p.chars().count() >= 4 && RUNS.iter().any(|r| r.contains(p.as_str())) {
        return "That password is just a run of keys.".into();
    }
    String::new()
}

/// How a version 2 file's key is locked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lock {
    /// "PBKDF2-SHA256".
    pub kdf: String,
    /// PBKDF2 rounds.
    pub iterations: u64,
    /// base64url.
    pub salt: String,
    /// base64url.
    pub iv: String,
    /// The key (a JWK, as JSON) encrypted, base64url.
    pub ct: String,
}

/// The club a club's backup brings back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Club {
    /// Its code.
    pub code: String,
    /// Its name.
    pub name: String,
}

/// A backup file, read: its header, its key (version 1) or lock (version 2),
/// and the club it's for, if any.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Backup {
    /// 1 or 2.
    pub v: u32,
    /// The player ID it holds.
    pub id: String,
    /// "ed25519" or "p256".
    pub alg: String,
    /// The last sequence number used.
    pub seq: Value,
    /// Version 1: the private key (a JWK).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<Value>,
    /// Version 2: how the key is locked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock: Option<Lock>,
    /// The club, for a club's backup.
    pub club: Option<Club>,
}

impl Backup {
    /// What a version 2 file binds as associated data: its header,
    /// `{type, v: 2, id, alg, seq}` (canonicalized by the caller).
    pub fn head(&self) -> Value {
        json!({"type": "ducy-identity", "v": 2, "id": self.id, "alg": self.alg, "seq": self.seq})
    }
}

/// Reads backup `text`: a version 1 or 2 ducy identity file, its lock
/// within bounds. Why not, in words, if it isn't one.
pub fn read_backup(text: &str) -> Result<Backup, String> {
    let f: Value = serde_json::from_str(text).map_err(|_| NOT_A_FILE.to_string())?;
    if f.get("type").and_then(Value::as_str) != Some("ducy-identity") {
        return Err(NOT_A_FILE.into());
    }
    let v = f.get("v").and_then(Value::as_u64).unwrap_or(0);
    let alg = f.get("alg").and_then(Value::as_str).unwrap_or("");
    if !matches!(v, 1 | 2) || !matches!(alg, "ed25519" | "p256") {
        return Err(NOT_A_FILE.into());
    }
    let club = f
        .get("club")
        .filter(|c| c.get("code").is_some_and(Value::is_string))
        .map(|c| Club {
            code: c["code"].as_str().unwrap_or("").into(),
            name: c.get("name").and_then(Value::as_str).unwrap_or("").into(),
        });
    let mut b = Backup {
        v: v as u32,
        id: f.get("id").and_then(Value::as_str).unwrap_or("").into(),
        alg: alg.into(),
        seq: f.get("seq").cloned().unwrap_or(Value::Null),
        key: None,
        lock: None,
        club,
    };
    if v == 1 {
        b.key = Some(
            f.get("key")
                .filter(|k| k.is_object())
                .cloned()
                .ok_or(NOT_A_FILE)?,
        );
    } else {
        let lock: Lock = f
            .get("lock")
            .cloned()
            .and_then(|l| serde_json::from_value(l).ok())
            .ok_or(NOT_A_FILE)?;
        if lock.kdf != "PBKDF2-SHA256" || lock.iterations < 1 || lock.iterations > 10_000_000 {
            return Err(NOT_A_FILE.into());
        }
        b.lock = Some(lock);
    }
    Ok(b)
}

/// Whether backup `text` needs a password to open (a version 2 file).
pub fn needs_password(text: &str) -> bool {
    read_backup(text).is_ok_and(|b| b.v == 2)
}

/// A club's backup file: player ID backup `locked` (JSON) with the club
/// added.
pub fn with_club(locked: &str, code: &str, name: &str) -> Result<String, String> {
    let mut f: Value = serde_json::from_str(locked).map_err(|e| e.to_string())?;
    f["club"] = json!({"code": code, "name": name});
    Ok(f.to_string())
}

/// Checks that a backup, opened (its club, and the player ID its key holds,
/// `id`), would bring back club `code` for owner `owner`. Why not, in words
/// for the owner.
pub fn prove_club(club: Option<&Club>, id: &str, code: &str, owner: &str) -> Result<(), String> {
    let Some(c) = club else {
        return Err(
            "That’s a player ID backup, not this club’s. Choose the file you just saved.".into(),
        );
    };
    if c.code != code {
        let which = if c.name.is_empty() { &c.code } else { &c.name };
        return Err(format!(
            "That’s the backup of another club ({which}). Choose this club’s file."
        ));
    }
    if id != owner {
        return Err("That backup holds another player ID, not the one that owns this club.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_for_new_backups() {
        for ok in [
            "correct horse",
            "tea at 4 on the porch",
            "Wq9#rLm2!vXa",
            "night owls rule the felt",
        ] {
            assert_eq!(password_problem(ok, "Night Owls"), "", "{ok}");
        }
        let why = |pw: &str| password_problem(pw, "Night Owls");
        assert!(why("owls").contains("at least 12"));
        assert!(why("Night Owls 1!").contains("club’s name"));
        assert!(why("nightowlsnightowls").contains("club’s name"));
        assert!(why("pokerclub2026").contains("too common"));
        assert!(why("P@ssw0rd!!!!").contains("too common"));
        assert!(why("abababababab").contains("repeated"));
        assert!(why("aaaaaaaaaaaaab").contains("repeated"));
        assert!(why("abcdefghijklm").contains("run of keys"));
        assert!(why("qwertyuiopas").contains("run of keys"));
    }

    #[test]
    fn reading_a_backup_and_proving_a_clubs() {
        let v2 = r#"{"type":"ducy-identity","v":2,"id":"abcdEFGH","alg":"ed25519","seq":5,
            "lock":{"kdf":"PBKDF2-SHA256","iterations":600000,"salt":"s","iv":"i","ct":"c"}}"#;
        let b = read_backup(v2).unwrap();
        assert_eq!((b.v, b.id.as_str(), b.club.clone()), (2, "abcdEFGH", None));
        assert!(needs_password(v2));
        assert_eq!(b.head()["v"], 2);
        let club = with_club(v2, "AAAA-AAAA", "Night Owls").unwrap();
        let b = read_backup(&club).unwrap();
        assert_eq!(b.club.as_ref().unwrap().name, "Night Owls");
        assert_eq!(
            prove_club(b.club.as_ref(), "abcdEFGH", "AAAA-AAAA", "abcdEFGH"),
            Ok(())
        );
        assert!(
            prove_club(b.club.as_ref(), "abcdEFGH", "BBBB-BBBB", "abcdEFGH")
                .unwrap_err()
                .contains("another club (Night Owls)")
        );
        assert!(
            prove_club(b.club.as_ref(), "abcdEFGH", "AAAA-AAAA", "zzzz")
                .unwrap_err()
                .contains("another player ID")
        );
        assert!(
            prove_club(None, "x", "AAAA-AAAA", "x")
                .unwrap_err()
                .contains("not this club’s")
        );
        let v1 =
            r#"{"type":"ducy-identity","v":1,"id":"x","alg":"p256","seq":0,"key":{"kty":"EC"}}"#;
        assert!(!needs_password(v1));
        assert!(read_backup(v1).unwrap().key.is_some());
        for bad in [
            "not a file",
            r#"{"type":"other"}"#,
            r#"{"type":"ducy-identity","v":3,"alg":"p256"}"#,
            r#"{"type":"ducy-identity","v":2,"alg":"p256","lock":{"kdf":"x","iterations":1,"salt":"","iv":"","ct":""}}"#,
        ] {
            assert_eq!(read_backup(bad).unwrap_err(), NOT_A_FILE, "{bad}");
        }
    }
}
