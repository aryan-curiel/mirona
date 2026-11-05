"""Build ~2 min test clips + reference transcripts from public datasets (via HF datasets-server).

  librispeech-clean  LibriSpeech test-clean, first utterances (speaker 6930)   CC-BY-4.0
  librispeech-other  LibriSpeech test-other, first utterances                  CC-BY-4.0
  ami-meeting        AMI ihm test, meeting EN2002a, time-ordered segments      CC-BY-4.0

Utterances are joined with 0.5 s silence into audio/<name>.wav (16 kHz mono s16) and
audio/<name>.txt. Usage: python3 -I fetch_audio.py <audio_dir>
"""

import json, os, subprocess, sys, tempfile, urllib.request

API = "https://datasets-server.huggingface.co/rows?dataset={}&config={}&split={}&offset={}&length=100"
TARGET_S = 120.0


def rows(dataset, config, split, offset):
    with urllib.request.urlopen(API.format(dataset, config, split, offset)) as r:
        return [x["row"] for x in json.load(r)["rows"]]


def build(out_dir, name, utts):
    """utts: list of (audio_url, text). Downloads, converts, concatenates."""
    with tempfile.TemporaryDirectory() as tmp:
        parts, texts, total = [], [], 0.0
        silence = os.path.join(tmp, "sil.wav")
        subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=16000:cl=mono",
                        "-t", "0.5", "-c:a", "pcm_s16le", silence], check=True)
        for i, (url, text) in enumerate(utts):
            if total >= TARGET_S:
                break
            raw, wav = os.path.join(tmp, f"{i}.raw"), os.path.join(tmp, f"{i}.wav")
            urllib.request.urlretrieve(url, raw)
            subprocess.run(["ffmpeg", "-v", "error", "-i", raw, "-ar", "16000", "-ac", "1",
                            "-c:a", "pcm_s16le", wav], check=True)
            total += os.path.getsize(wav) / 32000 + 0.5
            parts += [wav, silence]
            texts.append(text)
        lst = os.path.join(tmp, "list.txt")
        with open(lst, "w") as f:
            f.writelines(f"file '{p}'\n" for p in parts)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-f", "concat", "-safe", "0", "-i", lst,
                        "-c", "copy", os.path.join(out_dir, f"{name}.wav")], check=True)
        with open(os.path.join(out_dir, f"{name}.txt"), "w") as f:
            f.write(" ".join(texts).lower() + "\n")
        print(f"{name}: {len(texts)} utterances, {total:.1f} s")


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    for name, cfg, split in [("librispeech-clean", "clean", "test"),
                             ("librispeech-other", "other", "test")]:
        r = rows("openslr/librispeech_asr", cfg, split, 0)
        build(out, name, [(x["audio"][0]["src"], x["text"]) for x in r])

    # AMI segments come unordered; take 400, sort by time, keep a window from the first
    # segment past 5 min, skip backchannels (< 3 words) that carry little WER signal.
    ami = [x for off in range(0, 400, 100) for x in rows("edinburghcstr/ami", "ihm", "test", off)]
    ami = sorted((x for x in ami if x["meeting_id"] == "EN2002a" and x["begin_time"] > 300
                  and len(x["text"].split()) >= 3), key=lambda x: x["begin_time"])
    build(out, "ami-meeting", [(x["audio"][0]["src"], x["text"]) for x in ami])


if __name__ == "__main__":
    main()
