#!/usr/bin/env python3
"""A minimal external ducy-play bot.

Reads one JSON message per line on stdin and answers "act" messages with one
JSON line on stdout. Run a match against it with:

    cargo run -p ducy-play --example bot_match -- python3 ducy-play/examples/bots/simple_bot.py

Strategy: raise the minimum with a pair in the hole (Hold'em) or two paired
hole cards (Omaha), otherwise check or call, and fold to bets bigger than
half the pot.
"""
import json
import sys


def ranks(cards):
    # "As Kd" -> ["A", "K"]
    return [card[0] for card in cards.split()]


def decide(obs):
    legal = obs["legal"]
    hole = ranks(obs["hole_cards"])
    paired = len(set(hole)) < len(hole)

    if paired:
        for kind in ("raise", "bet"):
            if legal.get(kind):
                return {"action": kind, "amount": legal[kind]["min_to"]}
    if legal["can_check"]:
        return {"action": "check"}
    if legal["call"] is not None and legal["call"] <= obs["pot"] / 2:
        return {"action": "call"}
    return {"action": "fold"}


def main():
    for line in sys.stdin:
        message = json.loads(line)
        if message["type"] == "act":
            reply = decide(message["observation"])
            reply["id"] = message["id"]
            print(json.dumps(reply), flush=True)
        # "hand_over" messages carry the result; this bot ignores them.


if __name__ == "__main__":
    main()
