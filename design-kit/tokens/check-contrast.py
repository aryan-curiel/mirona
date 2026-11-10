"""Check WCAG text contrast (>= 4.5:1) for both themes in tokens.css.

Glass is approximated by blending each surface over the backdrop base color.
Run: python3 design-kit/tokens/check-contrast.py   (exits 1 on any failure)
"""
import re, sys, pathlib

css = pathlib.Path(__file__).with_name("tokens.css").read_text()

def block(selector):
    m = re.search(re.escape(selector) + r"\s*\{(.*?)\n\}", css, re.S)
    return dict(re.findall(r"--(mi-[\w-]+):\s*([^;]+);", m.group(1)))

def rgba(v):
    v = v.strip()
    if v.startswith("#"):
        return tuple(int(v[i:i + 2], 16) for i in (1, 3, 5)) + (1.0,)
    r, g, b, a = re.findall(r"[\d.]+", v)[:4]
    return int(r), int(g), int(b), float(a)

def over(top, base):
    a = top[3]
    return tuple(top[i] * a + base[i] * (1 - a) for i in range(3)) + (1.0,)

def lum(c):
    def ch(x):
        x /= 255
        return x / 12.92 if x <= 0.04045 else ((x + 0.055) / 1.055) ** 2.4
    return 0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])

def ratio(a, b):
    la, lb = sorted((lum(a), lum(b)), reverse=True)
    return (la + 0.05) / (lb + 0.05)

TEXT = ["ink", "ink-2", "ink-3", "muted", "accent-strong"]
SURFACES = ["glass-panel-bg", "glass-card-bg", "glass-row-bg", "glass-row-selected-bg"]
CHIPS = [("accent-strong", "accent-tint"), ("decision", "decision-tint"),
         ("question", "question-tint"), ("question", "review-tint"),
         ("neutral-text", "neutral-tint"), ("sensitive", "sensitive-tint")]
CHIPS += [(f"tag-{n}", f"tag-{n}-tint") for n in range(1, 9)]  # tag groups (NGR-202)

failed = 0
light = block(":root")
for name, theme in [("light", light), ("dark", {**light, **block('[data-theme="dark"]')})]:
    t = lambda k: theme["mi-" + k]
    base = rgba(t("backdrop-base"))
    pairs = [(fg, s, rgba(t(fg)), over(rgba(t(s)), base)) for fg in TEXT for s in SURFACES]
    pairs += [(fg, bg, rgba(t(fg)), over(rgba(t(bg)), base)) for fg, bg in CHIPS]
    for fg, bg, a, b in pairs:
        r = ratio(a, b)
        if r < 4.5:
            failed += 1
            print(f"FAIL {name}: {fg} on {bg} = {r:.2f}")
    print(f"{name}: {len(pairs)} pairs checked")
sys.exit(1 if failed else 0)
