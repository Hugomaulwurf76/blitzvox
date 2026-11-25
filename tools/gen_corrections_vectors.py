"""
Erzeugt Testvektoren für src-tauri/src/corrections.rs aus der Python-Version 1.0.8.

Die Rust-Tests vergleichen ihre Ergebnisse mit diesen Werten, damit die
Textverarbeitung exakt wie in der alten Version arbeitet.

Aufruf (aus dem Projektordner, schreibt nichts in "BlitzVox alt"):
    python -B tools/gen_corrections_vectors.py
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "BlitzVox alt"))

from blitzvox.services import corrections as c  # noqa: E402

SENTENCES = [
    "Hallo Komma wie geht es dir Fragezeichen",
    "Hallo, Komma, wie geht es dir? Fragezeichen",
    "Ich komme morgen Punkt neuer Absatz Liebe Grüße Komma neue Zeile Christian",
    "der Punkt ist wichtig Punkt",
    "Setz hier ein Komma hin Ausrufezeichen",
    "Klammer auf siehe oben Klammer zu Punkt",
    "Er sagte Anführungszeichen unten Hallo Anführungszeichen oben Punkt",
    "Er sagte Anführungszeichen auf Hallo Anführungszeichen zu.",
    "Das ist toll Gedankenstrich wirklich toll Punkt",
    "E Bindestrich Mail schreiben Punkt",
    "Und dann Auslassungspunkte nichts mehr",
    "Liste Doppelpunkt Äpfel Komma Birnen Semikolon Ende Punkt",
    "ein Strichpunkt reicht Strichpunkt oder?",
    "Kein Befehl in diesem Satz.",
    "KOMMA in Großbuchstaben NEUE   ZEILE geht auch",
    "  Punkt am Anfang",
    "über Punkt ändern Komma öfter Fragezeichen ärger",
    "Zeile neue Zeile\tneue Zeile Ende",
    "Kommandant Punktlandung Fragezeichenartig",
    "Das kostet 3 Punkt 5 Euro",
    "Hallo zusammen. Komma. und weiter",
    "passt. ? oder nicht",
    "morgen , wenn es passt..",
    "vergessen.. aber ... bleibt?!",
    "Wirklich ; ? ja",
    "Text mit Klammer ( innen ) und “ Zitat ”",
    "",
    "   ",
]

REPLACEMENTS = {
    "Blitz Text": "BlitzVox",
    "Blitz": "Blitze",
    "scuzzy": "SCSI",
    "Groq": "Groq",
    "o'neil": "O'Neil",
    "Über ich": "Überich",
}

REPLACE_TEXTS = [
    "Der Blitz Text ist schnell, Blitz!",
    "Scuzzy-Festplatte und SCUZZY Controller",
    "Blitztext bleibt, aber Blitz wird ersetzt",
    "o'neil und O'NEIL sowie über ich",
    "BlitzBlitz Blitz_Text Blitz",
]

EDITS = [
    ("Die Scuzzy-Festplatte ist kaputt", "Die SCSI-Festplatte ist kaputt"),
    ("Hallo Welt", "hallo welt!"),
    ("Ich nutze Blitz Text täglich", "Ich nutze BlitzVox täglich"),
    ("eins zwei drei vier fünf sechs", "eins A B C D sechs"),
    ("eins zwei drei", "eins zwei drei vier"),
    ("das ist der beste Test", "das ist ein sehr guter Test"),
    ("O'Neil spielt Groq", "O’Neil spielt Grok"),
    ("a b a b a b", "a c a c a b"),
    ("", "neu"),
    ("Mehrfach Fehler Fehler", "Mehrfach Fähler Fähler"),
]

SAFETY = [
    ("Hallo Komma wie geht es dir Fragezeichen", "Hallo, wie geht es dir?"),
    ("Hallo Komma wie geht es dir", "Hallo, wie geht es Ihnen?"),
    ("Punkt neue Zeile Ende", "Ende"),
    ("Das ist gut", "Das ist gut."),
    ("Das ist gut", ""),
    ("Das ist gut", "Das gut ist"),
    ("Er sagte Anführungszeichen unten Hallo Anführungszeichen oben", "Er sagte „Hallo“"),
    ("neue Absatz test", "test"),
    ("Komma Komma Punkt", "   "),
    ("Text", "Text und mehr"),
]

PROMPT = c.STT_PUNCTUATION_PROMPT
ECHO = [
    ("Hallo Komma wie geht es dir", PROMPT),
    ("hallo komma", PROMPT),
    ("Liebe Grüße Komma neue Zeile Christian", PROMPT),
    ("Ganz anderer Text hier", PROMPT),
    ("", PROMPT),
    ("eins zwei drei", "null eins zwei drei vier"),
]

VOCABS = [
    [],
    ["SCSI", "BlitzVox", "Groq"],
    ["Wort%02d-mit-etwas-Länge" % i for i in range(30)],
]


def main() -> None:
    out = {
        "spoken_punctuation": [[t, c.apply_spoken_punctuation(t)] for t in SENTENCES],
        "tidy_punctuation": [[t, c.tidy_punctuation(t)] for t in SENTENCES],
        "has_spoken_punctuation": [[t, c.has_spoken_punctuation(t)] for t in SENTENCES],
        "replacements": REPLACEMENTS,
        "apply_replacements": [
            [t, c.apply_replacements(t, {"replacements": REPLACEMENTS, "vocabulary": []})]
            for t in REPLACE_TEXTS
        ],
        "suggest_from_edit": [[a, b, c.suggest_from_edit(a, b)] for a, b in EDITS],
        "llm_output_is_safe": [[a, b, c.llm_output_is_safe(a, b)] for a, b in SAFETY],
        "is_prompt_echo": [[t, p, c.is_prompt_echo(t, p)] for t, p in ECHO],
        "build_stt_prompt": [],
    }
    for vocab in VOCABS:
        for sp in (True, False):
            c.get_vocabulary = lambda v=vocab: list(v)
            out["build_stt_prompt"].append([sp, vocab, c.build_stt_prompt(sp)])

    target = ROOT / "src-tauri" / "tests" / "data" / "corrections_vectors.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.write_text(json.dumps(out, indent=2, ensure_ascii=False), encoding="utf-8")
    print(f"geschrieben: {target}")


if __name__ == "__main__":
    main()
