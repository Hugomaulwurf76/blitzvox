"""
Nachbearbeitung des erkannten Textes.

1. Diktierte Satzzeichen ("Komma", "Fragezeichen", "neue Zeile" ...) werden
   durch das Zeichen ersetzt.
2. Korrektur-Wörterbuch: lernt aus Nutzerkorrekturen (Korrekturfenster).
   - replacements: falsch erkannt -> richtig (wird nach der STT angewendet)
   - vocabulary:   richtige Schreibweisen; gehen als Hinweis an die STT
"""

import json
import re
import threading
from pathlib import Path

from .settings import _config_path

_LOCK = threading.Lock()


# ── Letzter Text (nur im Speicher, für das Korrekturfenster) ──────────────────

_last_text: str = ""


def set_last(text: str) -> None:
    global _last_text
    _last_text = text


def get_last() -> str:
    return _last_text


# ── Korrektur-Wörterbuch ──────────────────────────────────────────────────────

def _path() -> Path:
    return _config_path().parent / "corrections.json"


def load() -> dict:
    data = {"replacements": {}, "vocabulary": []}
    p = _path()
    if p.exists():
        try:
            with open(p, encoding="utf-8") as f:
                saved = json.load(f)
            data["replacements"].update(saved.get("replacements", {}))
            data["vocabulary"] = list(saved.get("vocabulary", []))
        except (OSError, ValueError):
            pass  # defekte Datei -> leeres Wörterbuch, Programm läuft weiter
    return data


def save(data: dict) -> None:
    with _LOCK:
        p = _path()
        tmp = p.with_suffix(".tmp")
        with open(tmp, "w", encoding="utf-8") as f:
            json.dump(data, f, indent=2, ensure_ascii=False)
        tmp.replace(p)  # atomar: keine halb geschriebene Datei


def add_replacement(wrong: str, right: str) -> None:
    wrong, right = wrong.strip(), right.strip()
    if not wrong or not right or wrong.lower() == right.lower():
        return
    data = load()
    data["replacements"][wrong] = right
    if right not in data["vocabulary"]:
        data["vocabulary"].append(right)
    save(data)


def remove_replacement(wrong: str) -> None:
    data = load()
    right = data["replacements"].pop(wrong, None)
    # Vokabel nur entfernen, wenn kein anderer Eintrag mehr darauf zeigt
    if right is not None and right not in data["replacements"].values():
        data["vocabulary"] = [v for v in data["vocabulary"] if v != right]
    save(data)


def get_vocabulary() -> list[str]:
    return load()["vocabulary"]


# Bindestrich trennt: aus "Scuzzy-Festplatte" -> "SCSI-Festplatte" wird
# "Scuzzy" -> "SCSI" gelernt (greift dann auch bei "Scuzzy-Controller")
_WORD = re.compile(r"\w+(?:['’]\w+)*")
_MAX_WORDS = 3   # mehr als 3 Wörter am Stück = Umformulierung, kein Hörfehler


def suggest_from_edit(original: str, corrected: str) -> list[tuple[str, str]]:
    """
    Vergleicht erkannten und korrigierten Text Wort für Wort und liefert
    Vorschläge (falsch, richtig). Speichert NICHTS - das entscheidet der Nutzer.

    Ignoriert werden:
    - reine Groß/Klein- und Satzzeichen-Änderungen
    - eingefügte oder gelöschte Wörter (kein eindeutiges Paar)
    - Ersetzungen mit mehr als _MAX_WORDS Wörtern (Umformulierung)
    """
    import difflib

    src = _WORD.findall(original)
    dst = _WORD.findall(corrected)
    sm = difflib.SequenceMatcher(
        a=[w.lower() for w in src], b=[w.lower() for w in dst], autojunk=False)

    suggestions: list[tuple[str, str]] = []
    for op, i1, i2, j1, j2 in sm.get_opcodes():
        if op != "replace":
            continue
        if (i2 - i1) > _MAX_WORDS or (j2 - j1) > _MAX_WORDS:
            continue
        wrong = " ".join(src[i1:i2])
        right = " ".join(dst[j1:j2])
        if wrong.lower() != right.lower() and (wrong, right) not in suggestions:
            suggestions.append((wrong, right))
    return suggestions


def apply_replacements(text: str, data: dict | None = None) -> str:
    """Ersetzt bekannte Fehlerkennungen (ganze Wörter, Groß/Klein egal)."""
    data = data or load()
    reps = data["replacements"]
    # Längste Einträge zuerst, damit "Blitz Text" vor "Blitz" greift
    for wrong in sorted(reps, key=len, reverse=True):
        right = reps[wrong]
        pattern = r"(?<!\w)" + re.escape(wrong) + r"(?!\w)"
        text = re.sub(pattern, lambda m: right, text, flags=re.IGNORECASE)
    return text


# ── Diktierte Satzzeichen ─────────────────────────────────────────────────────

# Gesprochener Befehl -> Zeichen. Mehrwortbefehle mit einfachem Leerzeichen.
SPOKEN_PUNCTUATION: dict[str, str] = {
    "komma":                    ",",
    "punkt":                    ".",
    "fragezeichen":             "?",
    "ausrufezeichen":           "!",
    "doppelpunkt":              ":",
    "semikolon":                ";",
    "strichpunkt":              ";",
    "bindestrich":              "-",
    "gedankenstrich":           "–",
    "auslassungspunkte":        "…",
    "neue zeile":               "\n",
    "neuer absatz":             "\n\n",
    "klammer auf":              "(",
    "klammer zu":               ")",
    "anführungszeichen auf":    "„",
    "anführungszeichen unten":  "„",
    "anführungszeichen zu":     "“",
    "anführungszeichen oben":   "“",
}

# Steht eines dieser Wörter direkt davor, ist das Wort als Nomen gemeint
# ("der Punkt", "ein Komma") und bleibt stehen.
_DETERMINERS = {
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
}

# Zeichen, vor denen kein Leerzeichen steht
_NO_SPACE_BEFORE = ",.;:!?)…“"
# Zeichen, nach denen kein Leerzeichen steht
_NO_SPACE_AFTER = "(„"


def _build_pattern() -> re.Pattern:
    phrases = sorted(SPOKEN_PUNCTUATION, key=len, reverse=True)
    alts = "|".join(r"\s+".join(map(re.escape, p.split())) for p in phrases)
    # Whisper setzt um diktierte Satzzeichen oft eigene Zeichen
    # ("Hallo, Komma, wie ..."). Die werden mit verschluckt.
    return re.compile(
        r"[ \t]*[,.;:!?]?[ \t]*(?<!\w)(" + alts + r")(?!\w)[,.;:!?]?",
        re.IGNORECASE,
    )


_PATTERN = _build_pattern()


def apply_spoken_punctuation(text: str) -> str:
    def _repl(m: re.Match) -> str:
        before = text[: m.start(1)].split()
        prev = before[-1].strip(",.;:!?").lower() if before else ""
        if prev in _DETERMINERS:
            return m.group(0)  # Nomen, nicht ersetzen
        phrase = " ".join(m.group(1).lower().split())
        return "\x00" + SPOKEN_PUNCTUATION[phrase] + "\x00"

    out = _PATTERN.sub(_repl, text)
    if "\x00" not in out:
        return text

    # Leerzeichen um die eingefügten Zeichen ordnen
    def _fix(m: re.Match) -> str:
        sym = m.group(1)
        if sym in ("\n", "\n\n"):
            return sym
        if sym in ("-",):
            return sym
        if sym == "–":
            return " – "
        if sym in _NO_SPACE_AFTER:
            return " " + sym
        return sym + " "

    out = re.sub(r"\s*\x00(.*?)\x00\s*", _fix, out, flags=re.DOTALL)

    # Aufräumen
    out = re.sub(r"[ \t]+([" + re.escape(_NO_SPACE_BEFORE) + r"])", r"\1", out)
    out = re.sub(r"([" + re.escape(_NO_SPACE_AFTER) + r"])[ \t]+", r"\1", out)
    out = re.sub(r"[ \t]*\n[ \t]*", "\n", out)
    out = re.sub(r"[ \t]{2,}", " ", out)
    # Nach Satzende großschreiben
    out = re.sub(r"([.!?]\s+|\n)([a-zäöü])", lambda m: m.group(1) + m.group(2).upper(), out)
    out = re.sub(r"^\s*([a-zäöü])", lambda m: m.group(1).upper(), out)
    return out.strip(" \t")


_DETECT = re.compile(
    r"(?<!\w)(" + "|".join(
        r"\s+".join(map(re.escape, p.split()))
        for p in sorted(SPOKEN_PUNCTUATION, key=len, reverse=True)
    ) + r")(?!\w)",
    re.IGNORECASE,
)


# Hinweis-Prompt für Whisper: zeigt Befehle als ausgeschriebene Wörter, damit
# Whisper sie stehen lässt statt sie zu verschlucken (v. a. "neue Zeile").
# Whisper befolgt keine Anweisungen, es imitiert nur den Stil des Prompts.
STT_PUNCTUATION_PROMPT = (
    "Hallo Komma wie geht es dir Fragezeichen neue Zeile "
    "Ich komme morgen Punkt neuer Absatz Liebe Grüße Komma neue Zeile Christian"
)


# Whisper (Groq/OpenAI) wertet nur ca. 224 Tokens Prompt aus -> Vokabeln begrenzen
_VOCAB_MAX_CHARS = 350


def build_stt_prompt(spoken_punctuation: bool) -> str:
    """
    Hinweis-Prompt für die Erkennung:
    - Satzzeichen-Befehle als Wörter (falls aktiviert)
    - gelernte Schreibweisen (neueste zuerst, begrenzt)
    """
    parts = []
    if spoken_punctuation:
        parts.append(STT_PUNCTUATION_PROMPT)
    words, used = [], 0
    for w in reversed(get_vocabulary()):
        if used + len(w) + 2 > _VOCAB_MAX_CHARS:
            break
        words.append(w)
        used += len(w) + 2
    if words:
        parts.append(" ".join(words))
    return " ".join(parts)


def is_prompt_echo(text: str, prompt: str) -> bool:
    """Whisper gibt bei Stille manchmal den Prompt zurück -> verwerfen."""
    t = " ".join(re.findall(r"\w+", text.lower()))
    p = " ".join(re.findall(r"\w+", prompt.lower()))
    return bool(t) and len(t.split()) >= 3 and t in p


def tidy_punctuation(text: str) -> str:
    """
    Räumt doppelte Satzzeichen auf, die durch Whispers eigene Zeichen plus
    diktierte Zeichen entstehen: "passt. ?" -> "passt?", "morgen , wenn" -> "morgen, wenn".
    "..." und "?!" bleiben unberührt (kein Leerzeichen dazwischen).
    """
    # Whisper-Zeichen vor einem nachfolgenden (diktierten) Zeichen entfernen
    text = re.sub(r"[.,;:][ \t]+(?=[?!.,;:])", "", text)
    # Genau zwei Punkte -> einer ("vergessen.." -> "vergessen."), "..." bleibt
    text = re.sub(r"(?<!\.)\.\.(?!\.)", ".", text)
    # Komma/Semikolon/Doppelpunkt direkt vor Satzende -> nur Satzende
    text = re.sub(r"[,;:](?=[.?!])", "", text)
    # Leerzeichen vor Satzzeichen entfernen
    text = re.sub(r"[ \t]+([?!.,;:)“])", r"\1", text)
    return text


def has_spoken_punctuation(text: str) -> bool:
    """Kommt überhaupt ein Satzzeichen-Wort vor? (spart unnötige LLM-Aufrufe)"""
    return bool(_DETECT.search(text))


# ── Prüfung der LLM-Antwort ───────────────────────────────────────────────────

_PHRASE_TOKENS = [tuple(p.split()) for p in SPOKEN_PUNCTUATION]


def _tokens(text: str) -> list[str]:
    return re.findall(r"\w+", text.lower())


def _is_command_run(run: list[str]) -> bool:
    """Besteht die Wortfolge vollständig aus Satzzeichen-Befehlen?"""
    if not run:
        return True
    for phrase in _PHRASE_TOKENS:
        n = len(phrase)
        if tuple(run[:n]) == phrase and _is_command_run(run[n:]):
            return True
    return False


def llm_output_is_safe(original: str, output: str) -> bool:
    """
    True, wenn das LLM nur Satzzeichen-Befehle entfernt hat.
    Satzzeichen und Groß/Klein werden ignoriert. Jede andere Wortänderung -> False.
    """
    if not output.strip():
        return False
    src, dst = _tokens(original), _tokens(output)
    removed_runs, run, j = [], [], 0
    for tok in src:
        if j < len(dst) and dst[j] == tok:
            if run:
                removed_runs.append(run)
                run = []
            j += 1
        else:
            run.append(tok)
    if run:
        removed_runs.append(run)
    if j != len(dst):
        return False  # LLM hat Wörter hinzugefügt oder umgestellt
    return all(_is_command_run(r) for r in removed_runs)
