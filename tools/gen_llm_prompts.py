"""
Erzeugt src-tauri/tests/data/llm_prompts.json aus der Python-Version 1.0.8,
damit die Rust-Tests die Prompts wörtlich vergleichen können.

Aufruf (aus dem Projektordner):  python -B tools/gen_llm_prompts.py
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.dont_write_bytecode = True
sys.path.insert(0, str(ROOT / "BlitzVox alt"))

from blitzvox.services import llm  # noqa: E402
from blitzvox.services.settings import (  # noqa: E402
    DampfAblassenSettings, TextImprovementSettings,
)

IMPROVE = [
    dict(system_prompt="", context="", tone="neutral", custom_terms=[]),
    dict(system_prompt="", context="", tone="formal", custom_terms=[]),
    dict(system_prompt="", context="Mail an Kunden", tone="casual", custom_terms=["BlitzVox", "SCSI"]),
    dict(system_prompt="", context="", tone="unbekannt", custom_terms=[]),
    dict(system_prompt="Eigener Prompt.", context="K", tone="formal", custom_terms=["X"]),
]

out = {
    "PUNCTUATION_HINT": llm.PUNCTUATION_HINT,
    "PUNCTUATION_PROMPT": llm.PUNCTUATION_PROMPT,
    "DAMPF_DEFAULT": DampfAblassenSettings().system_prompt,
    "emoji": [[d, llm._build_emoji_prompt(d)] for d in ("wenig", "mittel", "viel", "anders")],
    "improve": [[s, llm._build_improve_prompt(TextImprovementSettings(**s))] for s in IMPROVE],
}
target = ROOT / "src-tauri" / "tests" / "data" / "llm_prompts.json"
target.write_text(json.dumps(out, indent=2, ensure_ascii=False), encoding="utf-8")
print(f"geschrieben: {target}")
