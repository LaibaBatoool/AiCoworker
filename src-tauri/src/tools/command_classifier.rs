use crate::permission::PermissionTier;

/// Classifies a shell command's risk tier. Rule-based on purpose (not
/// AI-based): the decision must be deterministic, auditable and
/// unit-tested, same "code decides, not the model" idea as the rest of
/// the permission system.
///
/// Order of rules:
///   1. PRIVILEGED if anything in the command looks dangerous: it points
///      outside the workspace, contains a URL, uses a destructive /
///      system / network / installer program, starts a nested shell, or
///      changes the clock.
///   2. SAFE only if it passes a strict DEFAULT-DENY allowlist: one
///      single read-only command, no chaining/redirects/env vars, and
///      the program + arguments are on the list below.
///   3. Everything else is MUTATING (runs inside the workspace with a
///      snapshot taken first, no confirmation).
///
/// Found in review (Oct 2026): the old version matched the safe list
/// with starts_with, so `cd .. && rmdir /s /q outside` and
/// `echo pwned > ..\outside\x` were classified SAFE, and `rmdir /s`
/// slipped past the "rd /s" pattern. Shell commands never go through
/// Workspace::resolve, so this classifier is the only thing standing
/// between execute_command and the rest of the disk.
///
/// Design choice: false positives are allowed, false negatives are not.
/// Privileged program names are matched as ANY word, so e.g.
/// `findstr del notes.txt` asks for confirmation. Annoying, but it fails
/// in the safe direction.
///
/// Known limits (documented, not hidden):
///   - a script run through an interpreter (`python script.py`,
///     `node app.js`) is MUTATING and can still do anything the script
///     says; only obvious `..`/drive/URL text in the command is caught.
///   - `npm install` / `pip install` with no URL still reach the public
///     package registries.
///   Real shell confinement needs an OS sandbox; this is best-effort
///   pattern matching.
pub fn classify_command_risk(command: &str) -> PermissionTier {
    if is_privileged(command) {
        return PermissionTier::Privileged;
    }
    if is_safe(command) {
        return PermissionTier::Safe;
    }
    PermissionTier::Mutating
}

// ---------------------------------------------------------------
// Lists
// ---------------------------------------------------------------

/// Programs that are privileged no matter where they appear in the
/// command (so they can't be hidden after `&&`, inside `for ... do`,
/// or inside `if exist x ...`).
const PRIVILEGED_PROGRAMS: &[&str] = &[
    // deleting / wiping
    "del", "erase", "rd", "rmdir", "rm", "remove-item", "sdelete", "cipher", "robocopy",
    "format", "diskpart", "mkfs", "dd", "fsutil", "defrag", "chkdsk", "label", "subst",
    // system state, users, services, registry, power
    "shutdown", "restart-computer", "stop-computer", "reg", "regedit", "bcdedit",
    "takeown", "icacls", "cacls", "attrib", "chmod", "chown", "runas", "sudo",
    "sc", "schtasks", "wmic", "vssadmin", "wbadmin", "setx", "assoc", "ftype",
    "taskkill", "stop-process", "powercfg", "netsh", "mklink",
    // installing software
    "msiexec", "winget", "choco", "scoop",
    // clock (the `date` incident) — date/time themselves are handled separately
    "set-date", "set-timezone", "tzutil", "w32tm",
    // network (fetch_url is privileged, so the shell must not be a way around it)
    "curl", "wget", "invoke-webrequest", "iwr", "invoke-restmethod", "irm",
    "bitsadmin", "certutil", "ftp", "tftp", "ssh", "scp", "sftp", "nc", "ncat",
    "telnet", "net", "nslookup",
    // nested shells / launchers: their inner code is opaque (e.g. -EncodedCommand)
    // and `start` launches processes that escape the timeout + kill-tree
    "cmd", "powershell", "pwsh", "wsl", "bash", "sh", "wscript", "cscript",
    "mshta", "rundll32", "regsvr32", "start", "explorer",
];

/// git subcommands that talk to the network or can push data out.
const GIT_NETWORK_SUBCOMMANDS: &[&str] = &[
    "clone", "fetch", "pull", "push", "remote", "submodule", "ls-remote",
];

/// Read-only programs allowed in the SAFE tier (still subject to the
/// "single command, no metacharacters" rule). Anything not listed is
/// at least MUTATING — that's what default-deny means.
const SAFE_PROGRAMS: &[&str] = &[
    "dir", "type", "tree", "where", "whoami", "hostname", "ver", "vol",
    "echo", "cd", "chdir", "findstr", "find", "fc", "pwd", "ls", "cat",
];

/// Read-only git subcommands allowed in the SAFE tier.
const SAFE_GIT_SUBCOMMANDS: &[&str] = &["status", "diff", "log", "show"];

/// Programs allowed in the SAFE tier ONLY as a bare version check,
/// e.g. `node -v`, `python --version`, `go version`.
const VERSION_PROGRAMS: &[&str] = &[
    "node", "npm", "python", "python3", "py", "pip", "cargo", "rustc",
    "java", "go", "dotnet", "git",
];
const VERSION_FLAGS: &[&str] = &["-v", "--version", "-version", "version"];

/// Characters that chain commands, redirect output, group commands or
/// escape characters in cmd.exe. Any of these = never SAFE.
const SHELL_METACHARS: &[char] = &['&', '|', '>', '<', '^', ';', '(', ')', '`', '\n', '\r', '%', '!'];

// ---------------------------------------------------------------
// Tokenizing
// ---------------------------------------------------------------

/// Lowercases and strips carets (cmd's escape character: `d^el` runs `del`).
fn normalise(command: &str) -> String {
    command.to_lowercase().replace('^', "")
}

/// Splits into "words": whitespace, quotes, cmd separators, and
/// chaining/redirect/grouping characters all break words.
fn words(normalised: &str) -> Vec<String> {
    normalised
        .split(|c: char| {
            c.is_whitespace()
                || matches!(c, '"' | '\'' | ',' | '=' | '&' | '|' | '>' | '<' | ';' | '(' | ')' | '@' | '`')
        })
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

/// Every word we should check: the normal split, PLUS a second split
/// with quotes deleted first. cmd glues `"de"l` back into `del`, so the
/// quote-deleted version is what cmd actually sees.
fn all_words(normalised: &str) -> Vec<String> {
    let mut out = words(normalised);
    let no_quotes: String = normalised.chars().filter(|c| *c != '"' && *c != '\'').collect();
    out.extend(words(&no_quotes));
    out
}

/// `C:\Windows\System32\CMD.EXE` -> `cmd`, `del.exe` -> `del`.
fn program_name(word: &str) -> &str {
    let base = word.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(word);
    for ext in [".exe", ".com", ".bat", ".cmd", ".ps1"] {
        if let Some(stripped) = base.strip_suffix(ext) {
            return stripped;
        }
    }
    base
}

/// Splits into chained segments (`a && b | c` -> "a ", " b ", " c").
fn segments(normalised: &str) -> Vec<&str> {
    normalised
        .split(|c: char| matches!(c, '&' | '|' | ';' | '\n' | '\r' | '(' | ')'))
        .collect()
}

// ---------------------------------------------------------------
// Rule 1: privileged
// ---------------------------------------------------------------

/// Is this path component a parent reference? `..` obviously, but also
/// `...`, `.. .` style names: Windows trims trailing dots/spaces from
/// path components, so anything made only of dots (2+) is suspicious.
fn is_dots_component(part: &str) -> bool {
    part.len() >= 2 && part.chars().all(|c| c == '.')
}

/// Does this word point somewhere that might be outside the workspace?
fn escapes_workspace(word: &str) -> bool {
    if word.split(|c| c == '\\' || c == '/').any(is_dots_component) {
        return true;
    }
    let b = word.as_bytes();
    // drive paths: c:  c:\x  c:x
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return true;
    }
    // drive path embedded in an argument, e.g. /out:c:\x
    if word.contains(":\\") {
        return true;
    }
    if let Some(i) = word.find(":/") {
        // URLs (://) are handled by has_url below
        if !word[i..].starts_with("://") {
            return true;
        }
    }
    // UNC paths \\server\share and //server/share, and root-relative \x
    if word.starts_with("\\\\") || word.starts_with("//") || word.starts_with('\\') {
        return true;
    }
    false
}

/// Any URL in a command means network access (pip/npm/git/python can all
/// fetch a URL), which would bypass fetch_url's SSRF checks.
fn has_url(normalised: &str) -> bool {
    normalised.contains("://")
}

/// %VAR% expansion can turn into an absolute path (%USERPROFILE%,
/// %SYSTEMROOT%, ...), and %~ / %1 / %* are batch argument forms.
fn uses_env_expansion(normalised: &str) -> bool {
    let b = normalised.as_bytes();
    b.windows(2).any(|w| {
        w[0] == b'%' && (w[1].is_ascii_alphanumeric() || w[1] == b'_' || w[1] == b'~' || w[1] == b'*')
    })
}

fn is_privileged(command: &str) -> bool {
    let norm = normalise(command);
    let ws = all_words(&norm);

    if ws.iter().any(|w| escapes_workspace(w)) {
        return true;
    }
    if has_url(&norm) || uses_env_expansion(&norm) {
        return true;
    }
    if ws.iter().any(|w| PRIVILEGED_PROGRAMS.contains(&program_name(w))) {
        return true;
    }

    // git network subcommands: `git push`, `git -C x fetch`, ...
    for (i, w) in ws.iter().enumerate() {
        if program_name(w) == "git"
            && ws[i + 1..].iter().take(4).any(|s| GIT_NETWORK_SUBCOMMANDS.contains(&s.as_str()))
        {
            return true;
        }
    }

    // `date` / `time`: on Windows, without /t these drop into an
    // interactive "Enter the new date" prompt that sets the system clock.
    // Found live — see Key Findings. Checked PER SEGMENT, so the /t on
    // one command can't cover a bare `time` chained after it.
    for seg in segments(&norm) {
        let seg_words = all_words(seg);
        let Some(first) = seg_words.first() else { continue };
        if matches!(program_name(first), "date" | "time") && !seg_words.iter().any(|w| w == "/t") {
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------
// Rule 2: safe (default-deny allowlist)
// ---------------------------------------------------------------

fn is_safe(command: &str) -> bool {
    if command.contains(SHELL_METACHARS) {
        return false; // chaining, redirects, grouping, escapes, env vars
    }
    let norm = normalise(command);
    let ws = words(&norm);
    let Some(first) = ws.first() else { return false };
    let prog = program_name(first);
    let rest = &ws[1..];

    if (prog == "date" || prog == "time") && rest.len() == 1 && rest[0] == "/t" {
        return true;
    }

    if prog == "git" {
        let Some(sub) = rest.first() else { return false };
        // the subcommand must come straight after `git` (no `-c key=val`
        // tricks), and no --output (git diff/log can write files with it)
        return (SAFE_GIT_SUBCOMMANDS.contains(&sub.as_str())
            && !rest.iter().any(|a| a.starts_with("--output") || a.starts_with("--ext-diff")))
            || (rest.len() == 1 && VERSION_FLAGS.contains(&sub.as_str()));
    }

    if VERSION_PROGRAMS.contains(&prog) {
        return rest.len() == 1 && VERSION_FLAGS.contains(&rest[0].as_str());
    }

    // Exact program-name match, NOT starts_with (old bug: "ls" matched
    // "lsass_dump.exe").
    SAFE_PROGRAMS.contains(&prog)
}

// ---------------------------------------------------------------
// Tests: `cargo test command_classifier`
// ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use PermissionTier::*;

    fn check(cases: &[&str], want: PermissionTier) {
        for c in cases {
            assert_eq!(classify_command_risk(c), want, "command: {}", c);
        }
    }

    #[test]
    fn review_escapes_are_now_privileged() {
        // every command from the review that used to be Safe/Mutating
        check(
            &[
                "cd .. && rmdir /s /q outside",
                "echo pwned > ..\\outside\\canary.txt",
                "dir & del /q ..\\*",
                "rmdir /s /q ..",
                "del ..\\outside\\canary.txt",
                "powershell Remove-Item -Recurse -Force ..",
                "rm -rf ..",
            ],
            Privileged,
        );
    }

    #[test]
    fn path_escapes_are_privileged() {
        check(
            &[
                "type ..\\secret.txt",
                "dir ..",
                "cd ..",
                "cd ...\\x",
                "move a.txt ...",
                "type C:\\Windows\\win.ini",
                "dir c:",
                "copy notes.txt \\\\server\\share\\x",
                "type \\Windows\\win.ini",
                "copy a.txt %USERPROFILE%\\Desktop\\a.txt",
                "python script.py --out=c:\\x.txt",
                "type sub/../../outside/x",
                "python -c \"import shutil;shutil.rmtree('../outside')\"",
            ],
            Privileged,
        );
    }

    #[test]
    fn hidden_or_obfuscated_destruction_is_privileged() {
        check(
            &[
                "d^el notes.txt",               // caret escape
                "\"del\" notes.txt",            // quoted program name
                "\"de\"l notes.txt",            // split quotes, cmd glues them back
                "echo hi && del notes.txt",     // chained after a safe command
                "for %f in (*.txt) do del %f",  // hidden in a loop
                "if exist a.txt del a.txt",     // hidden in a condition
                "C:\\Windows\\System32\\cmd.exe /c del x",
                "del.exe notes.txt",
                "erase notes.txt",
                "rd /s /q build",
                "rmdir build",
                "attrib +h notes.txt",
                "mklink /j link C:\\",          // junction escape
            ],
            Privileged,
        );
    }

    #[test]
    fn network_installers_and_nested_shells_are_privileged() {
        check(
            &[
                "curl http://127.0.0.1/secret",
                "curl.exe -o x http://example.com",
                "pip install --index-url http://127.0.0.1/secret x", // SSRF via package manager
                "npm install http://127.0.0.1/x",
                "python fetch.py http://169.254.169.254/latest",
                "winget install x",
                "msiexec /i x.msi",
                "powershell -EncodedCommand ZABlAGwA",
                "pwsh -c Get-ChildItem",
                "cmd /c dir",
                "start notepad",
                "git push origin main",
                "git clone https://github.com/x/y",
                "git -C sub fetch",
                "certutil -urlcache -f http://x/a a",
                "net user",
            ],
            Privileged,
        );
    }

    #[test]
    fn clock_commands() {
        check(
            &["date", "time", "date -d @1700000000", "echo x & date", "date /t & time", "time /t && date"],
            Privileged,
        );
        check(&["date /t", "time /t"], Safe);
    }

    #[test]
    fn allowlisted_read_only_commands_are_safe() {
        check(
            &[
                "dir",
                "dir /b sub",
                "type notes.txt",
                "type \"my notes.txt\"",
                "type notes.v1.txt",            // dots inside a name are fine
                "type .gitignore",
                "tree",
                "where python",
                "whoami",
                "echo hello",
                "cd",
                "findstr hello notes.txt",
                "git status",
                "git diff",
                "git log --oneline -5",
                "node -v",
                "python --version",
                "cargo --version",
                "go version",
                "git --version",
            ],
            Safe,
        );
    }

    #[test]
    fn unknown_or_writing_commands_default_to_mutating() {
        check(
            &[
                "python script.py",             // runs code inside the workspace
                "npm install",
                "mkdir build",
                "copy a.txt b.txt",
                "move a.txt sub\\a.txt",
                "echo hello > notes.txt",       // redirect inside the workspace
                "type a.txt | sort",            // piping = not a single command
                "lsass_dump.exe",               // old bug: matched the "ls" prefix
                "git commit -m wip",            // writes, so not safe
                "git diff --output=patch.txt",  // git diff can write a file
                "git -c core.pager=less status",// subcommand must follow git directly
                "node app.js",
                "ping localhost",
            ],
            Mutating,
        );
    }
}