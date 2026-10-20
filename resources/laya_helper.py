"""File Sorter's helper for Laya (https://huggingface.co/convaiinnovations/laya).

Loads the multilingual checkpoint once and answers questions: one JSON request per line
on standard input, one JSON answer per line on standard output. Ends when its input ends
(File Sorter ended or let it go).

    {"id": 1, "states": ["…"], "questions": {…}}  ->  {"id": 1, "answers": [{…}]}

With --download it only fetches the checkpoint (and checks it answers), then ends.
"""
import json
import os
import sys

REPO = "convaiinnovations/laya"
SUBFOLDER = "multilingual"


def say(message):
    sys.stdout.write(json.dumps(message, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def main():
    # Running in the background: leave the computer's other cores alone.
    threads = max(1, min(4, (os.cpu_count() or 2) // 2))
    try:
        import torch

        torch.set_num_threads(threads)
    except Exception:
        pass
    import laya

    agent = laya.load(REPO, subfolder=SUBFOLDER, device="cpu")
    if "--download" in sys.argv:
        answer = agent.system_one(
            "Abgabe der Serie 5 bis Freitag, 7. November 2026, 12:00.",
            {"due": {"type": "noul", "instructions": "Does the text name a deadline?"}},
        )
        say({"ready": True, "check": answer.get("answers", {})})
        return
    say({"ready": True})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        request = {}
        try:
            request = json.loads(line)
            states = request["states"]
            questions = request["questions"]
            max_len = request.get("max_len")
            if len(states) == 1:
                results = [agent.system_one(states[0], questions, max_len=max_len)]
            else:
                results = agent.predict_batch(states, questions, batch_size=8, max_len=max_len)
            say({"id": request.get("id"), "answers": [r.get("answers", {}) for r in results]})
        except Exception as error:  # one bad request must not end the helper
            say({"id": request.get("id"), "error": f"{type(error).__name__}: {error}"[:400]})


if __name__ == "__main__":
    main()
