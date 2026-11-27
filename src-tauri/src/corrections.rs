//! Nachbearbeitung des erkannten Textes – 1:1-Nachbau von
//! `BlitzVox alt/blitzvox/services/corrections.py`.
//!
//! 1. Diktierte Satzzeichen („Komma“, „Fragezeichen“, „neue Zeile“ …) werden
//!    durch das Zeichen ersetzt.
//! 2. Korrektur-Wörterbuch `corrections.json`: lernt aus Nutzerkorrekturen.
//!    - replacements: falsch erkannt -> richtig (wird nach der STT angewendet)
//!    - vocabulary:   richtige Schreibweisen; gehen als Hinweis an die STT
//!
//! Die regulären Ausdrücke entsprechen denen der Python-Version. Weil sie
//! Lookbehind/Lookahead brauchen, wird `fancy-regex` verwendet.

// Wird erst mit den Workflows und der Oberfläche vollständig benutzt.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use fancy_regex::{Captures, Regex};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::settings::{config_dir, to_python_json};

// ── Letzter Text (nur im Speicher, für das Korrekturfenster) ─────────────────

static LAST_TEXT: Mutex<String> = Mutex::new(String::new());

pub fn set_last(text: &str) {
    *LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner()) = text.to_string();
}

pub fn get_last() -> String {
    LAST_TEXT.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ── Korrektur-Wörterbuch ─────────────────────────────────────────────────────

/// Inhalt von `corrections.json`. Die Reihenfolge der Einträge bleibt
/// erhalten (wie bei einem Python-dict).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct CorrectionData {
    pub replacements: Map<String, Value>,
    pub vocabulary: Vec<String>,
}

impl CorrectionData {
    /// Ersetzungen als (falsch, richtig); Einträge ohne Text werden übergangen.
    fn replacement_pairs(&self) -> Vec<(&str, &str)> {
        self.replacements
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|r| (k.as_str(), r)))
            .collect()
    }
}

static SAVE_LOCK: Mutex<()> = Mutex::new(());

pub fn corrections_path() -> io::Result<PathBuf> {
    Ok(config_dir()?.join("corrections.json"))
}

/// Liest das Wörterbuch. Eine defekte Datei ergibt ein leeres Wörterbuch,
/// das Programm läuft weiter (wie `load()` in corrections.py).
pub fn load_from(path: &Path) -> CorrectionData {
    let Ok(bytes) = fs::read(path) else { return CorrectionData::default() };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(v) => from_value(v),
        Err(_) => CorrectionData::default(),
    }
}

/// Wörterbuch aus einem JSON-Wert (Datei oder Sicherung). Fehlt etwas oder
/// hat es den falschen Typ, bleibt der Teil leer.
pub fn from_value(value: Value) -> CorrectionData {
    let mut data = CorrectionData::default();
    let Value::Object(mut saved) = value else { return data };
    if let Some(Value::Object(reps)) = saved.remove("replacements") {
        data.replacements.extend(reps);
    }
    if let Some(Value::Array(voc)) = saved.remove("vocabulary") {
        data.vocabulary = voc.into_iter().filter_map(|v| v.as_str().map(String::from)).collect();
    }
    data
}

pub fn load() -> CorrectionData {
    match corrections_path() {
        Ok(p) => load_from(&p),
        Err(_) => CorrectionData::default(),
    }
}

/// Speichert atomar über eine Zwischendatei `corrections.tmp`.
pub fn save_to(path: &Path, data: &CorrectionData) -> io::Result<()> {
    let _guard = SAVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, to_python_json(data)?)?;
    fs::rename(&tmp, path) // ersetzt die alte Datei in einem Schritt
}

pub fn save(data: &CorrectionData) -> io::Result<()> {
    save_to(&corrections_path()?, data)
}

pub fn add_replacement_in(path: &Path, wrong: &str, right: &str) -> io::Result<()> {
    let (wrong, right) = (py_strip(wrong), py_strip(right));
    if wrong.is_empty() || right.is_empty() || wrong.to_lowercase() == right.to_lowercase() {
        return Ok(());
    }
    let mut data = load_from(path);
    data.replacements.insert(wrong.to_string(), Value::String(right.to_string()));
    if !data.vocabulary.iter().any(|v| v == right) {
        data.vocabulary.push(right.to_string());
    }
    save_to(path, &data)
}

pub fn add_replacement(wrong: &str, right: &str) -> io::Result<()> {
    add_replacement_in(&corrections_path()?, wrong, right)
}

pub fn remove_replacement_in(path: &Path, wrong: &str) -> io::Result<()> {
    let mut data = load_from(path);
    // shift_remove: Reihenfolge der übrigen Einträge bleibt wie in Python erhalten
    let right = data.replacements.shift_remove(wrong);
    // Vokabel nur entfernen, wenn kein anderer Eintrag mehr darauf zeigt
    if let Some(right) = right {
        if !data.replacements.values().any(|v| *v == right) {
            data.vocabulary.retain(|v| right.as_str() != Some(v.as_str()));
        }
    }
    save_to(path, &data)
}

pub fn remove_replacement(wrong: &str) -> io::Result<()> {
    remove_replacement_in(&corrections_path()?, wrong)
}

pub fn get_vocabulary() -> Vec<String> {
    load().vocabulary
}

/// `str.strip()` ohne Argument: entfernt Unicode-Leerraum an beiden Enden.
fn py_strip(s: &str) -> &str {
    s.trim_matches(char::is_whitespace)
}

/// Pythons `len()` zählt Zeichen (Codepoints), nicht Bytes.
fn py_len(s: &str) -> usize {
    s.chars().count()
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("fester regulärer Ausdruck ist gültig")
}

/// Bindestrich trennt: aus „Scuzzy-Festplatte“ -> „SCSI-Festplatte“ wird
/// „Scuzzy“ -> „SCSI“ gelernt (greift dann auch bei „Scuzzy-Controller“).
static WORD: LazyLock<Regex> = LazyLock::new(|| re(r"\w+(?:['’]\w+)*"));
/// Mehr als 3 Wörter am Stück = Umformulierung, kein Hörfehler.
const MAX_WORDS: usize = 3;

fn find_all(regex: &Regex, text: &str) -> Vec<String> {
    regex.find_iter(text).filter_map(|m| m.ok()).map(|m| m.as_str().to_string()).collect()
}

/// Vergleicht erkannten und korrigierten Text Wort für Wort und liefert
/// Vorschläge (falsch, richtig). Speichert nichts – das entscheidet der Nutzer.
///
/// Ignoriert werden reine Groß/Klein- und Satzzeichen-Änderungen, eingefügte
/// oder gelöschte Wörter und Ersetzungen mit mehr als `MAX_WORDS` Wörtern.
pub fn suggest_from_edit(original: &str, corrected: &str) -> Vec<(String, String)> {
    let src = find_all(&WORD, original);
    let dst = find_all(&WORD, corrected);
    let a: Vec<String> = src.iter().map(|w| w.to_lowercase()).collect();
    let b: Vec<String> = dst.iter().map(|w| w.to_lowercase()).collect();

    let mut suggestions: Vec<(String, String)> = Vec::new();
    for op in difflib::opcodes(&a, &b) {
        if op.tag != difflib::Tag::Replace {
            continue;
        }
        if op.i2 - op.i1 > MAX_WORDS || op.j2 - op.j1 > MAX_WORDS {
            continue;
        }
        let wrong = src[op.i1..op.i2].join(" ");
        let right = dst[op.j1..op.j2].join(" ");
        let pair = (wrong, right);
        if pair.0.to_lowercase() != pair.1.to_lowercase() && !suggestions.contains(&pair) {
            suggestions.push(pair);
        }
    }
    suggestions
}

/// Ersetzt bekannte Fehlerkennungen (ganze Wörter, Groß/Klein egal).
pub fn apply_replacements_with(text: &str, data: &CorrectionData) -> String {
    let mut reps = data.replacement_pairs();
    // Längste Einträge zuerst, damit „Blitz Text“ vor „Blitz“ greift
    // (stabile Sortierung wie Pythons sorted()).
    reps.sort_by_key(|(wrong, _)| std::cmp::Reverse(py_len(wrong)));
    let mut text = text.to_string();
    for (wrong, right) in reps {
        let pattern = format!(r"(?i)(?<!\w){}(?!\w)", fancy_regex::escape(wrong));
        let Ok(regex) = Regex::new(&pattern) else { continue };
        text = regex.replace_all(&text, |_: &Captures<'_, str>| right.to_string()).into_owned();
    }
    text
}

/// Wie `apply_replacements(text)` ohne Daten: liest das Wörterbuch.
pub fn apply_replacements(text: &str) -> String {
    apply_replacements_with(text, &load())
}

// ── Diktierte Satzzeichen ────────────────────────────────────────────────────

/// Gesprochener Befehl -> Zeichen (Reihenfolge wie in corrections.py).
pub const SPOKEN_PUNCTUATION: [(&str, &str); 18] = [
    ("komma", ","),
    ("punkt", "."),
    ("fragezeichen", "?"),
    ("ausrufezeichen", "!"),
    ("doppelpunkt", ":"),
    ("semikolon", ";"),
    ("strichpunkt", ";"),
    ("bindestrich", "-"),
    ("gedankenstrich", "–"),
    ("auslassungspunkte", "…"),
    ("neue zeile", "\n"),
    ("neuer absatz", "\n\n"),
    ("klammer auf", "("),
    ("klammer zu", ")"),
    ("anführungszeichen auf", "„"),
    ("anführungszeichen unten", "„"),
    ("anführungszeichen zu", "“"),
    ("anführungszeichen oben", "“"),
];

/// Steht eines dieser Wörter direkt davor, ist das Wort als Nomen gemeint
/// („der Punkt“, „ein Komma“) und bleibt stehen.
const DETERMINERS: &[&str] = &[
    "der", "die", "das", "den", "dem", "des",
    "ein", "eine", "einen", "einem", "einer", "eines",
    "kein", "keine", "keinen", "keinem", "keiner", "keines",
    "dieser", "diese", "dieses", "diesen", "diesem",
    "jeder", "jede", "jedes", "jeden", "jedem",
    "welcher", "welche", "welches", "welchen", "welchem",
    "mein", "meine", "meinen", "dein", "deine", "deinen",
    "sein", "seine", "seinen", "ihr", "ihre", "ihren",
    "unser", "unsere", "unseren", "euer", "eure", "euren",
    "zum", "beim", "im", "am", "vom",
];

/// Zeichen, nach denen kein Leerzeichen steht
const NO_SPACE_AFTER: &str = "(„";

fn lookup_punctuation(phrase: &str) -> Option<&'static str> {
    SPOKEN_PUNCTUATION.iter().find(|(p, _)| *p == phrase).map(|(_, s)| *s)
}

/// Alternativen aller Befehle, längste zuerst, Wörter durch `\s+` getrennt.
fn phrase_alternatives() -> String {
    let mut phrases: Vec<&str> = SPOKEN_PUNCTUATION.iter().map(|(p, _)| *p).collect();
    phrases.sort_by_key(|p| std::cmp::Reverse(py_len(p)));
    phrases
        .iter()
        .map(|p| p.split_whitespace().map(fancy_regex::escape).collect::<Vec<_>>().join(r"\s+"))
        .collect::<Vec<_>>()
        .join("|")
}

/// Whisper setzt um diktierte Satzzeichen oft eigene Zeichen
/// („Hallo, Komma, wie …“). Die werden mit verschluckt.
static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    re(&format!(
        r"(?i)[ \t]*[,.;:!?]?[ \t]*(?<!\w)({})(?!\w)[,.;:!?]?",
        phrase_alternatives()
    ))
});

static DETECT: LazyLock<Regex> =
    LazyLock::new(|| re(&format!(r"(?i)(?<!\w)({})(?!\w)", phrase_alternatives())));

static FIX: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)\s*\x00(.*?)\x00\s*"));
static SPACE_BEFORE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]+([,.;:!?)…“])"));
static SPACE_AFTER: LazyLock<Regex> = LazyLock::new(|| re(r"([(„])[ \t]+"));
static AROUND_NEWLINE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]*\n[ \t]*"));
static MULTI_SPACE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]{2,}"));
static AFTER_SENTENCE: LazyLock<Regex> = LazyLock::new(|| re(r"([.!?]\s+|\n)([a-zäöü])"));
static TEXT_START: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*([a-zäöü])"));

fn group<'t>(c: &Captures<'t, str>, i: usize) -> &'t str {
    c.get(i).map(|m| m.as_str()).unwrap_or("")
}

pub fn apply_spoken_punctuation(text: &str) -> String {
    let out = PATTERN
        .replace_all(text, |c: &Captures<'_, str>| {
            let start = c.get(1).map(|m| m.start()).unwrap_or(0);
            let prev = text[..start]
                .split_whitespace()
                .last()
                .map(|w| w.trim_matches(|ch| ",.;:!?".contains(ch)).to_lowercase())
                .unwrap_or_default();
            if DETERMINERS.contains(&prev.as_str()) {
                return group(c, 0).to_string(); // Nomen, nicht ersetzen
            }
            let phrase = group(c, 1).to_lowercase().split_whitespace().collect::<Vec<_>>().join(" ");
            match lookup_punctuation(&phrase) {
                Some(sym) => format!("\x00{sym}\x00"),
                None => group(c, 0).to_string(),
            }
        })
        .into_owned();
    if !out.contains('\x00') {
        return text.to_string();
    }

    // Leerzeichen um die eingefügten Zeichen ordnen
    let out = FIX
        .replace_all(&out, |c: &Captures<'_, str>| {
            let sym = group(c, 1);
            match sym {
                "\n" | "\n\n" | "-" => sym.to_string(),
                "–" => " – ".to_string(),
                s if NO_SPACE_AFTER.contains(s) => format!(" {s}"),
                s => format!("{s} "),
            }
        })
        .into_owned();

    // Aufräumen
    let out = SPACE_BEFORE.replace_all(&out, "$1").into_owned();
    let out = SPACE_AFTER.replace_all(&out, "$1").into_owned();
    let out = AROUND_NEWLINE.replace_all(&out, "\n").into_owned();
    let out = MULTI_SPACE.replace_all(&out, " ").into_owned();
    // Nach Satzende großschreiben
    let out = AFTER_SENTENCE
        .replace_all(&out, |c: &Captures<'_, str>| format!("{}{}", group(c, 1), group(c, 2).to_uppercase()))
        .into_owned();
    let out = TEXT_START.replace_all(&out, |c: &Captures<'_, str>| group(c, 1).to_uppercase()).into_owned();
    out.trim_matches(|ch| ch == ' ' || ch == '\t').to_string()
}

/// Hinweis-Prompt für Whisper: zeigt Befehle als ausgeschriebene Wörter, damit
/// Whisper sie stehen lässt statt sie zu verschlucken (v. a. „neue Zeile“).
/// Whisper befolgt keine Anweisungen, es imitiert nur den Stil des Prompts.
pub const STT_PUNCTUATION_PROMPT: &str = "Hallo Komma wie geht es dir Fragezeichen neue Zeile \
Ich komme morgen Punkt neuer Absatz Liebe Grüße Komma neue Zeile Christian";

/// Whisper (Groq/OpenAI) wertet nur ca. 224 Tokens Prompt aus -> Vokabeln begrenzen.
const VOCAB_MAX_CHARS: usize = 350;

/// Hinweis-Prompt für die Erkennung: Satzzeichen-Befehle als Wörter (falls
/// aktiviert) und gelernte Schreibweisen (neueste zuerst, begrenzt).
pub fn build_stt_prompt_with(spoken_punctuation: bool, vocabulary: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if spoken_punctuation {
        parts.push(STT_PUNCTUATION_PROMPT.to_string());
    }
    let mut words: Vec<&str> = Vec::new();
    let mut used = 0;
    for w in vocabulary.iter().rev() {
        if used + py_len(w) + 2 > VOCAB_MAX_CHARS {
            break;
        }
        words.push(w);
        used += py_len(w) + 2;
    }
    if !words.is_empty() {
        parts.push(words.join(" "));
    }
    parts.join(" ")
}

pub fn build_stt_prompt(spoken_punctuation: bool) -> String {
    build_stt_prompt_with(spoken_punctuation, &get_vocabulary())
}

static WORD_CHARS: LazyLock<Regex> = LazyLock::new(|| re(r"\w+"));

fn tokens(text: &str) -> Vec<String> {
    find_all(&WORD_CHARS, &text.to_lowercase())
}

/// Whisper gibt bei Stille manchmal den Prompt zurück -> verwerfen.
pub fn is_prompt_echo(text: &str, prompt: &str) -> bool {
    let t = tokens(text);
    let p = tokens(prompt).join(" ");
    !t.is_empty() && t.len() >= 3 && p.contains(&t.join(" "))
}

static TIDY_BEFORE_MARK: LazyLock<Regex> = LazyLock::new(|| re(r"[.,;:][ \t]+(?=[?!.,;:])"));
static TIDY_TWO_DOTS: LazyLock<Regex> = LazyLock::new(|| re(r"(?<!\.)\.\.(?!\.)"));
static TIDY_BEFORE_END: LazyLock<Regex> = LazyLock::new(|| re(r"[,;:](?=[.?!])"));
static TIDY_SPACE: LazyLock<Regex> = LazyLock::new(|| re(r"[ \t]+([?!.,;:)“])"));

/// Räumt doppelte Satzzeichen auf, die durch Whispers eigene Zeichen plus
/// diktierte Zeichen entstehen: „passt. ?“ -> „passt?“,
/// „morgen , wenn“ -> „morgen, wenn“. „...“ und „?!“ bleiben unberührt.
pub fn tidy_punctuation(text: &str) -> String {
    let text = TIDY_BEFORE_MARK.replace_all(text, "").into_owned();
    let text = TIDY_TWO_DOTS.replace_all(&text, ".").into_owned();
    let text = TIDY_BEFORE_END.replace_all(&text, "").into_owned();
    TIDY_SPACE.replace_all(&text, "$1").into_owned()
}

/// Kommt überhaupt ein Satzzeichen-Wort vor? (spart unnötige LLM-Aufrufe)
pub fn has_spoken_punctuation(text: &str) -> bool {
    DETECT.is_match(text).unwrap_or(false)
}

// ── Prüfung der LLM-Antwort ──────────────────────────────────────────────────

static PHRASE_TOKENS: LazyLock<Vec<Vec<&'static str>>> = LazyLock::new(|| {
    SPOKEN_PUNCTUATION.iter().map(|(p, _)| p.split_whitespace().collect()).collect()
});

/// Besteht die Wortfolge vollständig aus Satzzeichen-Befehlen?
fn is_command_run(run: &[String]) -> bool {
    if run.is_empty() {
        return true;
    }
    PHRASE_TOKENS.iter().any(|phrase| {
        let n = phrase.len();
        run.len() >= n
            && run[..n].iter().zip(phrase).all(|(a, b)| a == b)
            && is_command_run(&run[n..])
    })
}

/// True, wenn das LLM nur Satzzeichen-Befehle entfernt hat.
/// Satzzeichen und Groß/Klein werden ignoriert. Jede andere Wortänderung -> false.
pub fn llm_output_is_safe(original: &str, output: &str) -> bool {
    if py_strip(output).is_empty() {
        return false;
    }
    let (src, dst) = (tokens(original), tokens(output));
    let mut removed_runs: Vec<Vec<String>> = Vec::new();
    let mut run: Vec<String> = Vec::new();
    let mut j = 0;
    for tok in src {
        if j < dst.len() && dst[j] == tok {
            if !run.is_empty() {
                removed_runs.push(std::mem::take(&mut run));
            }
            j += 1;
        } else {
            run.push(tok);
        }
    }
    if !run.is_empty() {
        removed_runs.push(run);
    }
    if j != dst.len() {
        return false; // LLM hat Wörter hinzugefügt oder umgestellt
    }
    removed_runs.iter().all(|r| is_command_run(r))
}

// ── difflib.SequenceMatcher (ohne Junk, autojunk=False) ──────────────────────

/// Nachbau der von `suggest_from_edit` genutzten Teile von Pythons
/// `difflib.SequenceMatcher(autojunk=False)`, damit die Vorschläge exakt
/// gleich ausfallen.
mod difflib {
    use super::HashMap;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Tag {
        Replace,
        Delete,
        Insert,
        Equal,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Opcode {
        pub tag: Tag,
        pub i1: usize,
        pub i2: usize,
        pub j1: usize,
        pub j2: usize,
    }

    fn find_longest_match(
        a: &[String],
        b: &[String],
        b2j: &HashMap<&str, Vec<usize>>,
        alo: usize,
        ahi: usize,
        blo: usize,
        bhi: usize,
    ) -> (usize, usize, usize) {
        let (mut besti, mut bestj, mut bestsize) = (alo, blo, 0usize);
        let mut j2len: HashMap<usize, usize> = HashMap::new();
        for i in alo..ahi {
            let mut newj2len: HashMap<usize, usize> = HashMap::new();
            if let Some(js) = b2j.get(a[i].as_str()) {
                for &j in js {
                    if j < blo {
                        continue;
                    }
                    if j >= bhi {
                        break;
                    }
                    let k = j.checked_sub(1).and_then(|p| j2len.get(&p)).copied().unwrap_or(0) + 1;
                    newj2len.insert(j, k);
                    if k > bestsize {
                        besti = i + 1 - k;
                        bestj = j + 1 - k;
                        bestsize = k;
                    }
                }
            }
            j2len = newj2len;
        }
        // Ohne Junk erweitern diese Schleifen nur noch gleiche Nachbarn.
        while besti > alo && bestj > blo && a[besti - 1] == b[bestj - 1] {
            besti -= 1;
            bestj -= 1;
            bestsize += 1;
        }
        while besti + bestsize < ahi && bestj + bestsize < bhi && a[besti + bestsize] == b[bestj + bestsize] {
            bestsize += 1;
        }
        (besti, bestj, bestsize)
    }

    fn matching_blocks(a: &[String], b: &[String]) -> Vec<(usize, usize, usize)> {
        let mut b2j: HashMap<&str, Vec<usize>> = HashMap::new();
        for (j, elt) in b.iter().enumerate() {
            b2j.entry(elt.as_str()).or_default().push(j);
        }
        let (la, lb) = (a.len(), b.len());
        let mut queue = vec![(0, la, 0, lb)];
        let mut blocks = Vec::new();
        while let Some((alo, ahi, blo, bhi)) = queue.pop() {
            let (i, j, k) = find_longest_match(a, b, &b2j, alo, ahi, blo, bhi);
            if k > 0 {
                blocks.push((i, j, k));
                if alo < i && blo < j {
                    queue.push((alo, i, blo, j));
                }
                if i + k < ahi && j + k < bhi {
                    queue.push((i + k, ahi, j + k, bhi));
                }
            }
        }
        blocks.sort();
        // Benachbarte Blöcke zusammenfassen
        let mut merged = Vec::new();
        let (mut i1, mut j1, mut k1) = (0, 0, 0);
        for (i2, j2, k2) in blocks {
            if i1 + k1 == i2 && j1 + k1 == j2 {
                k1 += k2;
            } else {
                if k1 > 0 {
                    merged.push((i1, j1, k1));
                }
                (i1, j1, k1) = (i2, j2, k2);
            }
        }
        if k1 > 0 {
            merged.push((i1, j1, k1));
        }
        merged.push((la, lb, 0));
        merged
    }

    pub fn opcodes(a: &[String], b: &[String]) -> Vec<Opcode> {
        let (mut i, mut j) = (0, 0);
        let mut answer = Vec::new();
        for (ai, bj, size) in matching_blocks(a, b) {
            let tag = if i < ai && j < bj {
                Some(Tag::Replace)
            } else if i < ai {
                Some(Tag::Delete)
            } else if j < bj {
                Some(Tag::Insert)
            } else {
                None
            };
            if let Some(tag) = tag {
                answer.push(Opcode { tag, i1: i, i2: ai, j1: j, j2: bj });
            }
            (i, j) = (ai + size, bj + size);
            if size > 0 {
                answer.push(Opcode { tag: Tag::Equal, i1: ai, i2: i, j1: bj, j2: j });
            }
        }
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("blitzvox-korr-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn woerterbuch_lernen_und_entfernen() {
        let d = temp_dir("lernen");
        let p = d.join("corrections.json");
        add_replacement_in(&p, " Scuzzy ", "SCSI").unwrap();
        add_replacement_in(&p, "Blitz Text", "BlitzVox").unwrap();
        add_replacement_in(&p, "Blitztext", "BlitzVox").unwrap();
        add_replacement_in(&p, "gleich", "GLEICH").unwrap(); // nur Groß/Klein -> ignoriert
        let data = load_from(&p);
        assert_eq!(data.replacements.len(), 3);
        assert_eq!(data.vocabulary, vec!["SCSI", "BlitzVox"]);
        remove_replacement_in(&p, "Blitz Text").unwrap();
        assert_eq!(load_from(&p).vocabulary, vec!["SCSI", "BlitzVox"]); // noch benutzt
        remove_replacement_in(&p, "Blitztext").unwrap();
        assert_eq!(load_from(&p).vocabulary, vec!["SCSI"]);
        assert!(!d.join("corrections.tmp").exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// Vergleich mit Ergebnissen der Python-Version 1.0.8
    /// (erzeugt mit `tools/gen_corrections_vectors.py`).
    #[test]
    fn gleiche_ergebnisse_wie_python() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/corrections_vectors.json");
        let v: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let s = |x: &Value| x.as_str().unwrap().to_string();
        let mut fehler = Vec::new();

        for case in v["spoken_punctuation"].as_array().unwrap() {
            let got = apply_spoken_punctuation(&s(&case[0]));
            if got != s(&case[1]) {
                fehler.push(format!("spoken {:?}: {:?} != {:?}", case[0], got, case[1]));
            }
        }
        for case in v["tidy_punctuation"].as_array().unwrap() {
            let got = tidy_punctuation(&s(&case[0]));
            if got != s(&case[1]) {
                fehler.push(format!("tidy {:?}: {:?} != {:?}", case[0], got, case[1]));
            }
        }
        for case in v["has_spoken_punctuation"].as_array().unwrap() {
            let got = has_spoken_punctuation(&s(&case[0]));
            if got != case[1].as_bool().unwrap() {
                fehler.push(format!("detect {:?}: {got}", case[0]));
            }
        }
        let data = CorrectionData {
            replacements: v["replacements"].as_object().unwrap().clone(),
            vocabulary: Vec::new(),
        };
        for case in v["apply_replacements"].as_array().unwrap() {
            let got = apply_replacements_with(&s(&case[0]), &data);
            if got != s(&case[1]) {
                fehler.push(format!("replace {:?}: {:?} != {:?}", case[0], got, case[1]));
            }
        }
        for case in v["suggest_from_edit"].as_array().unwrap() {
            let got = suggest_from_edit(&s(&case[0]), &s(&case[1]));
            let want: Vec<(String, String)> = case[2]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| (s(&p[0]), s(&p[1])))
                .collect();
            if got != want {
                fehler.push(format!("suggest {:?}->{:?}: {got:?} != {want:?}", case[0], case[1]));
            }
        }
        for case in v["llm_output_is_safe"].as_array().unwrap() {
            let got = llm_output_is_safe(&s(&case[0]), &s(&case[1]));
            if got != case[2].as_bool().unwrap() {
                fehler.push(format!("safe {:?}->{:?}: {got}", case[0], case[1]));
            }
        }
        for case in v["is_prompt_echo"].as_array().unwrap() {
            let got = is_prompt_echo(&s(&case[0]), &s(&case[1]));
            if got != case[2].as_bool().unwrap() {
                fehler.push(format!("echo {:?}: {got}", case[0]));
            }
        }
        for case in v["build_stt_prompt"].as_array().unwrap() {
            let vocab: Vec<String> = case[1].as_array().unwrap().iter().map(s).collect();
            let got = build_stt_prompt_with(case[0].as_bool().unwrap(), &vocab);
            if got != s(&case[2]) {
                fehler.push(format!("prompt {}: {got:?} != {:?}", case[0], case[2]));
            }
        }
        assert!(fehler.is_empty(), "Abweichungen von Python:\n{}", fehler.join("\n"));
    }

    #[test]
    fn defekte_datei_ergibt_leeres_woerterbuch() {
        let d = temp_dir("defekt");
        let p = d.join("corrections.json");
        fs::write(&p, "[kaputt").unwrap();
        assert_eq!(load_from(&p), CorrectionData::default());
        let _ = fs::remove_dir_all(&d);
    }

    /// Kopie der echten corrections.json lesen und wieder schreiben:
    /// muss Byte für Byte gleich bleiben. Fehlt die Datei, wird übersprungen.
    #[test]
    fn echte_datei_bleibt_byte_gleich() {
        let Ok(real) = corrections_path() else { return };
        let Ok(original) = fs::read(&real) else { return };
        let d = temp_dir("echt");
        let copy = d.join("corrections.json");
        fs::write(&copy, &original).unwrap();
        let data = load_from(&copy);
        let out = d.join("neu.json");
        save_to(&out, &data).unwrap();
        assert!(fs::read(&out).unwrap() == original, "Ausgabe weicht von der Python-Datei ab");
        let _ = fs::remove_dir_all(&d);
    }
}
