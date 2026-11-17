#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

// Runs inside the lookup window on orbispatches.com / prosperopatches.com only.
// It waits for the page to render (the sites are JavaScript-driven), reads the game
// name and the patch versions from the DOM/network payloads, and hands the result back
// through the URL fragment (#psa=...), which Rust reads.
const INIT_JS: &str = r#"
(function () {
  if (window.top !== window) return;
  if (!/(^|\.)(orbispatches|prosperopatches)\.com$/.test(location.hostname)) return;
  var t0 = Date.now(), done = false, reqs = [], captured = [];
  var tid = ((location.pathname.match(/(CUSA|PPSA)\d{5}/i) || [''])[0]).toUpperCase();
  function put(o) { try { location.hash = 'psa=' + encodeURIComponent(JSON.stringify(o)); } catch (e) {} }
  function keep(url, text) {
    try { reqs.push(String(url).slice(0, 100)); } catch (e) {}
    if (/patch/i.test(String(url)) && text) captured.push({ url: String(url).slice(0, 100), text: String(text).slice(0, 30000) });
  }
  var of = window.fetch;
  if (of) window.fetch = function (input) {
    var url = (input && input.url) || input;
    var p = of.apply(this, arguments);
    try { p.then(function (r) { try { r.clone().text().then(function (tx) { keep(url, tx); }, function () {}); } catch (e) {} }, function () {}); } catch (e) {}
    return p;
  };
  var ox = XMLHttpRequest.prototype.open, os = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (m, u) { this.__psa = u; return ox.apply(this, arguments); };
  XMLHttpRequest.prototype.send = function () {
    var x = this;
    x.addEventListener('load', function () { try { keep(x.__psa, x.responseText); } catch (e) {} });
    return os.apply(this, arguments);
  };
  function versionsFrom(text) {
    var out = [];
    function walk(o, key) {
      if (o == null) return;
      if (typeof o === 'string' || typeof o === 'number') {
        var s = String(o);
        if (/^(app_?ver(sion)?|patch_?ver(sion)?|version|ver)$/i.test(key || '') && /^\d{1,2}\.\d{2}$/.test(s)) out.push(s);
        return;
      }
      if (Array.isArray(o)) { o.forEach(function (x) { walk(x, key); }); return; }
      if (typeof o === 'object') for (var k in o) walk(o[k], k);
    }
    try { walk(JSON.parse(text)); } catch (e) {}
    return out;
  }
  function uniq(a) { return a.filter(function (v, i) { return a.indexOf(v) === i; }); }
  function cleanName(s) {
    s = String(s || '').replace(/\s+/g, ' ').trim();
    if (tid) s = s.split(tid).join('');
    s = s.replace(/(ORBIS|PROSPERO)\s*Patches/ig, '');
    s = s.replace(/(ORBIS|PROSPERO)Patches?\.?com/ig, '');
    s = s.replace(/\bPatch Installer\b/ig, '');
    s = s.replace(/^[\s\-|–—:()\[\]]+|[\s\-|–—:()\[\]]+$/g, '');
    return s.trim();
  }
  // whole-string junk only (a real game may start with "Home" or "Patches")
  var BAD = /^(404|not found|page not found|just a moment|attention required)\b/i, BADEXACT = /^(home|patches|patch installer|error)$/i;
  function addCandidate(list, src, v, weight) {
    v = cleanName(v);
    if (!v || v.length < 2 || v.length > 160 || BAD.test(v) || BADEXACT.test(v)) return;
    list.push({ src: src, name: v, weight: weight || 0 });
  }
  function walkStrings(o, cb, key) {
    if (o == null) return;
    if (typeof o === 'string' || typeof o === 'number') { cb(String(o), key || ''); return; }
    if (Array.isArray(o)) { o.forEach(function (x) { walkStrings(x, cb, key); }); return; }
    if (typeof o === 'object') for (var k in o) walkStrings(o[k], cb, k);
  }
  function parseJsonText(text, src, weight, out) {
    try {
      var o = JSON.parse(text);
      walkStrings(o, function (s, key) {
        if (/^(name|title|headline|game|gamename|displayname|alternateName)$/i.test(key) && /[A-Z]{4}\d{5}/i.test(tid + ' ' + s)) {
          addCandidate(out, src + ':' + key, s, weight + 2);
        } else if (/^(name|title|headline|game|gamename|displayname|alternateName)$/i.test(key)) {
          addCandidate(out, src + ':' + key, s, weight + 1);
        }
      });
    } catch (e) {}
  }
  function candidates() {
    var c = [];
    function add(src, v, weight) { addCandidate(c, src, v, weight); }
    var selectors = [
      ['h3.h3-title, .h3-title', 30],
      ['h1', 28],
      ['h2', 24],
      ['.game-title', 25],
      ['.page-title', 20],
      ['[class*="title"]', 10]
    ];
    selectors.forEach(function (pair) {
      var nodes = document.querySelectorAll(pair[0]);
      for (var i = 0; i < nodes.length && i < 6; i++) add(pair[0], nodes[i].textContent, pair[1] - i);
    });
    var og = document.querySelector('meta[property="og:title"]'); if (og) add('og:title', og.getAttribute('content'), 18);
    var tw = document.querySelector('meta[name="twitter:title"]'); if (tw) add('twitter:title', tw.getAttribute('content'), 16);
    add('title', document.title, 4);
    var ld = document.querySelectorAll('script[type="application/ld+json"]');
    for (var i = 0; i < ld.length && i < 6; i++) parseJsonText(ld[i].textContent || '', 'ld+json[' + i + ']', 22 - i, c);
    var next = document.getElementById('__NEXT_DATA__'); if (next) parseJsonText(next.textContent || '', '__NEXT_DATA__', 24, c);
    return c.sort(function (a, b) {
      if (b.weight !== a.weight) return b.weight - a.weight;
      return a.name.length - b.name.length;
    });
  }
  function diag(cands, vs, full) {
    var d = 'url=' + location.pathname + ' title=' + document.title + '\nrequests=' + JSON.stringify(reqs.slice(-12)) +
      '\ncandidates=' + JSON.stringify(cands.slice(0, 8)) + '\nversions=' + JSON.stringify(vs);
    if (captured.length) d += '\ncaptured=' + captured.slice(0, 2).map(function (c) { return c.url + ' => ' + c.text.slice(0, full ? 1500 : 200); }).join(' | ');
    if (full) d += '\ntext=' + ((document.body && document.body.innerText) || '').slice(0, 600) + '\n---HTML---\n' + ((document.body && document.body.innerHTML) || '').slice(0, 1500);
    return d.slice(0, full ? 6000 : 700);
  }
  function scan() {
    if (done) return;
    var el = Date.now() - t0;
    var txt = ((document.body && document.body.innerText) || '').slice(0, 3000);
    var chall = /just a moment|verify you are human|checking your browser|cf-turnstile/i.test(document.title + ' ' + txt) ||
      !!document.querySelector('iframe[src*="challenges.cloudflare.com"]');
    if (chall && el > 3000) put({ status: 'challenge' });
    var vs = [];
    captured.forEach(function (c) { vs = vs.concat(versionsFrom(c.text)); });
    vs = uniq(vs);
    var cands = candidates();
    if (cands.length && !chall && (captured.length || el > 12000)) {
      done = true;
      put({ status: 'ok', name: cands[0].name, versions: vs, debug: diag(cands, vs, false) });
      return;
    }
    if (el > 30000 && !chall) {
      done = true;
      put({ status: 'notfound', debug: diag(cands, vs, true) });
      return;
    }
    setTimeout(scan, 400);
  }
  function start() { put({ status: 'boot' }); scan(); }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', start); else start();
})();
"#;
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct LookupResult {
    status: String,
    name: Option<String>,
    versions: Vec<String>,
    debug: Option<String>,
}

fn read_report(win: &tauri::WebviewWindow, tid: &str) -> Option<LookupResult> {
    let url = win.url().ok()?;
    // ignore the previous title's page while the new one is still loading
    if !url.path().to_uppercase().ends_with(tid) {
        return None;
    }
    let enc = url.fragment()?.strip_prefix("psa=")?;
    let json = percent_encoding::percent_decode_str(enc).decode_utf8().ok()?;
    serde_json::from_str::<LookupResult>(&json).ok()
}

#[tauri::command]
async fn lookup_title(app: AppHandle, tid: String, visible: Option<bool>) -> Result<LookupResult, String> {
    let visible = visible.unwrap_or(false);
    let tid = tid.to_uppercase();
    let valid = tid.is_ascii()
        && tid.len() == 9
        && (tid.starts_with("CUSA") || tid.starts_with("PPSA"))
        && tid[4..].chars().all(|c| c.is_ascii_digit());
    if !valid {
        return Err("invalid title id".into());
    }

    let host = if tid.starts_with("CUSA") { "orbispatches.com" } else { "prosperopatches.com" };
    let target = format!("https://{}/{}", host, tid);
    let parsed = tauri::Url::parse(&target).map_err(|e| e.to_string())?;
    let label = format!("lookup-{}-{}", tid, SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis());

    let win = WebviewWindowBuilder::new(&app, &label, WebviewUrl::External(parsed))
        .title("بررسی آنلاین — اگر تأیید انسانی خواست، یک‌بار انجامش دهید")
        .inner_size(900.0, 700.0)
        .visible(visible)
        .initialization_script(INIT_JS)
        .build()
        .map_err(|e| e.to_string())?;

    let start = Instant::now();
    let mut limit = Duration::from_secs(35);
    let mut challenge_seen = false;
    let mut shown_by_us = false; // opened only to let the user pass a check -> hide again afterwards
    let mut saw_boot = false; // did our script ever run + could we read its message back?
    loop {
        tokio::time::sleep(Duration::from_millis(400)).await;
        if let Some(rep) = read_report(&win, &tid) {
            match rep.status.as_str() {
                "boot" => saw_boot = true,
                "challenge" => {
                    saw_boot = true;
                    if !challenge_seen {
                        challenge_seen = true;
                        if !visible {
                            let _ = win.show();
                            let _ = win.set_focus();
                            shown_by_us = true;
                        }
                        limit = start.elapsed() + Duration::from_secs(240);
                    }
                }
                _ => {
                    if shown_by_us { let _ = win.hide(); }
                    let _ = win.close();
                    return Ok(rep);
                }
            }
        }
        if start.elapsed() > limit {
            if shown_by_us { let _ = win.hide(); }
            let last_url = win.url().map(|u| u.to_string()).unwrap_or_default();
            let note = if saw_boot {
                "script_ran=true (page loaded, but no final result before the time limit)"
            } else {
                "script_ran=false (our script never reported back: blocked page, no navigation, or the result channel does not work)"
            };
            let _ = win.close();
            return Ok(LookupResult {
                status: "timeout".into(),
                debug: Some(format!("{note}; last_url={}", &last_url.chars().take(160).collect::<String>())),
                ..Default::default()
            });
        }
    }
}
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://orbispatches.com/") || url.starts_with("https://prosperopatches.com/")) {
        return Err("blocked".into());
    }
    #[cfg(target_os = "windows")]
    let r = std::process::Command::new("explorer").arg(&url).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(&url).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(&url).spawn();
    r.map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn save_csv(app: AppHandle, filename: String, contents: String) -> Result<String, String> {
    let dir = app.path().download_dir().map_err(|e| e.to_string())?;
    let safe: String = filename
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '.')
        .collect();
    let path = dir.join(if safe.is_empty() { "export.csv".to_string() } else { safe });
    std::fs::write(&path, contents.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}


#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchiveFile {
    path: String,
    relative_path: String,
    name: String,
    size: u64,
    mtime_ms: u64,
    cid: Option<String>,
    file_type: Option<String>,
}

/// On Windows `canonicalize` returns verbatim paths (`\\?\C:\...`). Shell APIs such as
/// WScript.Shell and explorer.exe handle those badly, so strip the prefix.
fn plain_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return PathBuf::from(format!(r"\\{}", rest));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return PathBuf::from(rest.to_string());
    }
    p.to_path_buf()
}

fn supported_archive(path: &Path) -> bool {
    path.extension()
        .and_then(|v| v.to_str())
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "pkg" | "zip" | "rar" | "7z" | "iso" | "bin" | "exfat"))
        .unwrap_or(false)
}

fn inspect_pkg(path: &Path) -> (Option<String>, Option<String>) {
    if !path.extension().and_then(|v| v.to_str()).map(|v| v.eq_ignore_ascii_case("pkg")).unwrap_or(false) {
        return (None, None);
    }
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return (None, None),
    };
    let mut bytes = [0u8; 0x80];
    if file.read_exact(&mut bytes).is_err() || u32::from_be_bytes(bytes[0..4].try_into().unwrap()) != 0x7F434E54 {
        return (None, None);
    }

    let cid: String = bytes[0x40..0x64]
        .iter()
        .copied()
        .filter(|b| *b != 0)
        .map(char::from)
        .collect();
    let cid = if cid.is_empty() { None } else { Some(cid) };
    let kind = u32::from_be_bytes(bytes[0x74..0x78].try_into().unwrap());
    let file_type = match kind {
        0x1A => Some("Base Game".to_string()),
        0x1B | 0x1C => Some("DLC/Add-on".to_string()),
        0x1D => Some("Update".to_string()),
        _ => None,
    };
    (cid, file_type)
}

fn collect_archive_files(root: &Path, dir: &Path, output: &mut Vec<ArchiveFile>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("خواندن پوشه ممکن نشد: {} ({e})", dir.display()))?;
    for entry in entries {
        let entry = match entry {
            Ok(v) => v,
            Err(_) => continue,
        };
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(v) => v,
            Err(_) => continue,
        };
        // Do not follow symlinks: this avoids loops and keeps scanning within the selected folder.
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            if entry.file_name().to_string_lossy().eq_ignore_ascii_case("PS Archive Shortcuts") {
                continue;
            }
            let _ = collect_archive_files(root, &path, output);
            continue;
        }
        if !ft.is_file() || !supported_archive(&path) {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let (cid, file_type) = inspect_pkg(&path);
        let mtime_ms = metadata.modified().unwrap_or(UNIX_EPOCH)
            .duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64;
        output.push(ArchiveFile {
            path: path.to_string_lossy().into_owned(),
            relative_path: relative.to_string_lossy().replace('\\', "/"),
            name: path.file_name().unwrap_or_default().to_string_lossy().into_owned(),
            size: metadata.len(),
            mtime_ms,
            cid,
            file_type,
        });
    }
    Ok(())
}

#[tauri::command]
async fn list_archive_files(root: String) -> Result<Vec<ArchiveFile>, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Vec<ArchiveFile>, String> {
        let root = plain_path(&PathBuf::from(root).canonicalize().map_err(|e| format!("مسیر پوشه معتبر نیست: {e}"))?);
        if !root.is_dir() {
            return Err("مسیر انتخاب‌شده پوشه نیست".into());
        }
        let mut files = Vec::new();
        collect_archive_files(&root, &root, &mut files)?;
        files.sort_by(|a, b| a.relative_path.to_lowercase().cmp(&b.relative_path.to_lowercase()));
        Ok(files)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutInput {
    source_path: String,
    relative_path: String,
    game_name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutReport {
    created: usize,
    skipped: usize,
    errors: Vec<String>,
    output_dir: String,
}

/// WScript.Shell can only save shortcuts to paths made of characters in the system ANSI code page
/// (e.g. "METAL GEAR SOLID \u{394}" becomes "?"), so folder and shortcut names are folded to plain ASCII.
fn ascii_fold(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for c in input.chars() {
        if c.is_ascii() { out.push(c); continue; }
        let rep = match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' => "a", 'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' => "A",
            'ç' => "c", 'Ç' => "C", 'è' | 'é' | 'ê' | 'ë' | 'ē' => "e", 'È' | 'É' | 'Ê' | 'Ë' | 'Ē' => "E",
            'ì' | 'í' | 'î' | 'ï' | 'ī' => "i", 'Ì' | 'Í' | 'Î' | 'Ï' | 'Ī' => "I", 'ñ' => "n", 'Ñ' => "N",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ō' | 'ø' => "o", 'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ō' | 'Ø' => "O",
            'ù' | 'ú' | 'û' | 'ü' | 'ū' => "u", 'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ū' => "U",
            'ý' | 'ÿ' => "y", 'Ý' => "Y", 'ß' => "ss", 'æ' => "ae", 'Æ' => "AE", 'œ' => "oe", 'Œ' => "OE",
            'Δ' => "Delta", 'Σ' => "Sigma", 'Ω' => "Omega", 'α' => "alpha", 'β' => "beta", 'π' => "pi", 'λ' => "lambda", 'μ' => "mu",
            '™' | '®' | '©' | '“' | '”' | '«' | '»' => "", '’' | '‘' | '´' => "'", '–' | '—' | '−' => "-", '…' => "...", '×' => "x",
            '\u{00A0}' | '\u{2009}' | '\u{200B}' => " ",
            _ => "_",
        };
        out.push_str(rep);
    }
    out
}

fn safe_component(value: &str, max_chars: usize) -> String {
    let folded = ascii_fold(value);
    let mut result: String = folded.chars().map(|c| {
        if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c }
    }).take(max_chars).collect();
    result = result.trim().trim_end_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    if result.is_empty() { result = "Unknown game".to_string(); }
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"];
    let stem = result.split('.').next().unwrap_or("").to_ascii_uppercase();
    if reserved.contains(&stem.as_str()) { result.insert(0, '_'); }
    result
}

fn short_path_hash(value: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:08X}", (hash & 0xFFFF_FFFF) as u32)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ShortcutEntry {
    target_path: String,
    shortcut_path: String,
    relative_path: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PowerShellShortcutError {
    relative_path: String,
    message: String,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct PowerShellShortcutResult {
    created: usize,
    errors: Vec<PowerShellShortcutError>,
}

#[cfg(target_os = "windows")]
fn write_windows_shortcuts(output_dir: &Path, entries: &[ShortcutEntry]) -> Result<(usize, Vec<String>), String> {
    let mut created = 0usize;
    let mut errors: Vec<String> = Vec::new();
    for chunk in entries.chunks(200) {
        match write_windows_shortcuts_chunk(output_dir, chunk) {
            Ok((c, e)) => { created += c; errors.extend(e); }
            Err(e) => errors.push(e),
        }
    }
    Ok((created, errors))
}

#[cfg(target_os = "windows")]
fn write_windows_shortcuts_chunk(output_dir: &Path, entries: &[ShortcutEntry]) -> Result<(usize, Vec<String>), String> {
    if entries.is_empty() { return Ok((0, Vec::new())); }
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    let manifest = output_dir.join(format!(".psa-shortcuts-{}-{stamp}.json", std::process::id()));
    let payload = serde_json::to_vec(&serde_json::json!({ "items": entries })).map_err(|e| e.to_string())?;
    fs::write(&manifest, payload).map_err(|e| format!("ساخت فهرست شورتکات‌ها ناموفق بود: {e}"))?;

    // One PowerShell process creates all .lnk files in a batch, rather than launching
    // a new process for every archive item. Paths are read from a UTF-8 JSON manifest
    // so Persian and other Unicode folder/file names remain intact.
    let script = r#"$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$json = Get-Content -LiteralPath $env:PSA_MANIFEST_PATH -Raw -Encoding UTF8
$parsed = ConvertFrom-Json -InputObject $json
$items = @($parsed.items)
$shell = New-Object -ComObject WScript.Shell
$created = 0
$errors = @()
foreach ($item in $items) {
  try {
    $link = $shell.CreateShortcut([string]$item.shortcutPath)
    $link.TargetPath = [string]$item.targetPath
    # not Split-Path -LiteralPath -Parent: that combination fails on Windows PowerShell 5.1 ("Parameter set cannot be resolved")
    $link.WorkingDirectory = [System.IO.Path]::GetDirectoryName([string]$item.targetPath)
    $link.Description = 'PS Archive Manager shortcut'
    $link.Save()
    $created++
  } catch {
    $errors += [pscustomobject]@{ relativePath = [string]$item.relativePath; message = $_.Exception.Message }
  }
}
[pscustomobject]@{ created = $created; errors = @($errors) } | ConvertTo-Json -Depth 4 -Compress
"#;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000; // run in the background, no console window flashing
    let result = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script])
        .env("PSA_MANIFEST_PATH", &manifest)
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = fs::remove_file(&manifest);
    let output = result.map_err(|e| format!("اجرای PowerShell ممکن نشد: {e}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() { "ساخت شورتکات‌های ویندوز ناموفق بود".into() } else { detail });
    }
    let parsed: PowerShellShortcutResult = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("خواندن نتیجه ساخت شورتکات‌ها ناموفق بود: {e}; {}", String::from_utf8_lossy(&output.stdout)))?;
    let errors = parsed.errors.into_iter()
        .map(|e| format!("{}: {}", e.relative_path, e.message))
        .collect();
    Ok((parsed.created, errors))
}

#[cfg(not(target_os = "windows"))]
fn write_windows_shortcuts(_output_dir: &Path, _entries: &[ShortcutEntry]) -> Result<(usize, Vec<String>), String> {
    Err("ساخت شورتکات .lnk فقط در نسخه ویندوز پشتیبانی می‌شود".into())
}

#[tauri::command]
async fn create_game_shortcuts(root: String, items: Vec<ShortcutInput>) -> Result<ShortcutReport, String> {
    let root = plain_path(&PathBuf::from(root).canonicalize().map_err(|e| format!("مسیر آرشیو معتبر نیست: {e}"))?);
    if !root.is_dir() { return Err("مسیر آرشیو پوشه نیست".into()); }
    let output_dir = root.join("PS Archive Shortcuts");
    let mut report = ShortcutReport { created: 0, skipped: 0, errors: Vec::new(), output_dir: output_dir.to_string_lossy().into_owned() };
    if items.is_empty() { return Ok(report); }

    // .psa-index.json remembers which target each shortcut was created for, so two different
    // files with the same name never end up sharing (or silently skipping) a shortcut,
    // regardless of the order or subset of files in a given call.
    let index_path = output_dir.join(".psa-index.json");
    let mut index: HashMap<String, String> = fs::read(&index_path).ok()
        .and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut planned: HashMap<String, String> = HashMap::new();
    let mut pending: Vec<ShortcutEntry> = Vec::new();

    for item in items {
        let game_name = item.game_name.trim();
        if game_name.is_empty() { report.skipped += 1; continue; }
        let source = match PathBuf::from(&item.source_path).canonicalize() {
            Ok(p) => plain_path(&p),
            Err(_) => { report.errors.push(format!("فایل پیدا نشد: {}", item.relative_path)); continue; }
        };
        if !source.starts_with(&root) || !source.is_file() || !supported_archive(&source) {
            report.errors.push(format!("مسیر فایل معتبر نیست: {}", item.relative_path));
            continue;
        }
        let game_dir = output_dir.join(safe_component(game_name, 60));
        let original_name = source.file_name().unwrap_or_default().to_string_lossy().into_owned();
        // Windows MAX_PATH: keep the whole .lnk path under 260 characters.
        let dir_len = game_dir.to_string_lossy().chars().count();
        let room = 258usize.saturating_sub(dir_len + 1 + 4 + 11);
        if room < 12 {
            report.errors.push(format!("مسیر برای شورتکات خیلی طولانی است: {}", item.relative_path));
            continue;
        }
        let base_name = safe_component(&original_name, room.min(120));
        let target = source.to_string_lossy().into_owned();
        let candidates = [
            game_dir.join(format!("{base_name}.lnk")),
            game_dir.join(format!("{base_name} [{}].lnk", short_path_hash(&item.relative_path))),
        ];
        let mut chosen: Option<(PathBuf, bool)> = None; // (link path, already exists for this target)
        for cand in candidates {
            let key = cand.to_string_lossy().to_lowercase();
            match planned.get(&key).or_else(|| index.get(&key)) {
                Some(t) if *t == target => { chosen = Some((cand, true)); break; }
                Some(_) => continue,
                // made before the index existed: assume it is the right one
                None if cand.exists() => { chosen = Some((cand, true)); break; }
                None => { chosen = Some((cand, false)); break; }
            }
        }
        let (link_path, already) = match chosen {
            Some(v) => v,
            None => { report.errors.push(format!("برخورد نام شورتکات: {}", item.relative_path)); continue; }
        };
        if already { report.skipped += 1; continue; }
        if let Err(e) = fs::create_dir_all(&game_dir) {
            report.errors.push(format!("ساخت پوشه بازی ناموفق بود: {game_name} ({e})"));
            continue;
        }
        planned.insert(link_path.to_string_lossy().to_lowercase(), target.clone());
        pending.push(ShortcutEntry {
            target_path: target,
            shortcut_path: link_path.to_string_lossy().into_owned(),
            relative_path: item.relative_path,
        });
    }

    match write_windows_shortcuts(&output_dir, &pending) {
        Ok((created, errors)) => {
            report.created = created;
            report.errors.extend(errors);
        }
        Err(e) => report.errors.push(e),
    }
    if !pending.is_empty() {
        for e in &pending {
            if Path::new(&e.shortcut_path).exists() {
                index.insert(e.shortcut_path.to_lowercase(), e.target_path.clone());
            }
        }
        if let Ok(bytes) = serde_json::to_vec(&index) { let _ = fs::write(&index_path, bytes); }
    }
    report.errors.truncate(10);
    Ok(report)
}


#[tauri::command]
fn open_shortcuts_folder(root: String) -> Result<(), String> {
    let root = plain_path(&PathBuf::from(root).canonicalize().map_err(|e| e.to_string())?);
    if !root.is_dir() { return Err("مسیر آرشیو پوشه نیست".into()); }
    let folder = root.join("PS Archive Shortcuts");
    if !folder.is_dir() { return Err("هنوز پوشه شورتکات‌ها ساخته نشده است".into()); }
    #[cfg(target_os = "windows")]
    let result = Command::new("explorer.exe").arg(&folder).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(&folder).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(&folder).spawn();
    result.map(|_| ()).map_err(|e| e.to_string())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![lookup_title, open_url, save_csv, list_archive_files, create_game_shortcuts, open_shortcuts_folder])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" {
                    window.app_handle().exit(0);
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running application");
}
