#!/usr/bin/env python3
"""Draw the comparison charts from a results file.

    python3 bench/plot.py [bench/results/latest.json] [docs/bench]

One PNG per measurement and one overview, on a dark card so they read the same
on GitHub's light and dark themes. Values are written on the bars.
"""

import json
import pathlib
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
BG, FG, MUTED, GRID = "#14110b", "#f3ecd8", "#a89f86", "#2c261a"
COLOURS = {"zui": "#d8a81f", "3xui": "#6f7a86", "pasarguard": "#4d5b6b"}

plt.rcParams.update({
    "figure.facecolor": BG, "axes.facecolor": BG, "savefig.facecolor": BG,
    "text.color": FG, "axes.labelcolor": MUTED, "axes.edgecolor": GRID,
    "xtick.color": FG, "ytick.color": MUTED, "font.size": 11,
    "axes.spines.top": False, "axes.spines.right": False,
})


def load(path):
    data = json.loads(pathlib.Path(path).read_text())
    sizes = sorted({run["users"] for panel in data["panels"].values() for run in panel["runs"]})
    return data, sizes


def value(panel, users, getter):
    for run in panel["runs"]:
        if run["users"] == users and "error" not in run:
            try:
                return getter(run)
            except (KeyError, TypeError):
                return None
    return None


def bars(ax, data, sizes, getter, title, unit, fmt="{:,.0f}", lower_is_better=False, log=False):
    keys = list(data["panels"])
    width = 0.8 / len(keys)
    for i, key in enumerate(keys):
        panel = data["panels"][key]
        for j, users in enumerate(sizes):
            v = value(panel, users, getter)
            x = j + (i - (len(keys) - 1) / 2) * width
            colour = COLOURS.get(key, "#888")
            if v is None:
                ax.text(x, 0, "no result", rotation=90, ha="center", va="bottom", color=MUTED, fontsize=9)
                continue
            ax.bar(x, v, width * 0.92, color=colour, label=panel["name"] if j == 0 else None)
            ax.text(x, v, fmt.format(v), ha="center", va="bottom", color=FG, fontsize=9)
    ax.set_xticks(range(len(sizes)))
    ax.set_xticklabels([f"{n:,} users" for n in sizes])
    ax.set_title(title + ("  (lower is better)" if lower_is_better else "  (higher is better)"), loc="left", color=FG, fontsize=12)
    ax.set_ylabel(unit)
    ax.grid(axis="y", color=GRID, linewidth=0.6)
    ax.set_axisbelow(True)
    if log:
        ax.set_yscale("log")
    ax.margins(y=0.15)


CHARTS = [
    ("subscription-throughput", lambda r: r["subscription"]["rps"], "Subscription fetches per second", "requests / second", "{:,.0f}", False, False),
    ("subscription-latency", lambda r: r["subscription"]["p99_ms"], "Subscription latency, 99th percentile", "milliseconds", "{:,.1f}", True, False),
    ("user-list", lambda r: r["user_list"]["rps"], "Dashboard user list (50 per page)", "requests / second", "{:,.0f}", False, False),
    ("memory", lambda r: r["idle_loaded"]["rss_mb"], "Memory at rest, users loaded", "MB", "{:,.0f}", True, False),
    ("startup", lambda r: r["start_loaded_s"], "Time to start with the users stored", "seconds", "{:,.2f}", True, False),
    ("cpu-per-request", lambda r: r["subscription"]["cpu_ms_per_request"], "CPU per subscription fetch", "CPU-milliseconds", "{:,.2f}", True, False),
]


def main():
    source = sys.argv[1] if len(sys.argv) > 1 else ROOT / "bench" / "results" / "latest.json"
    out = pathlib.Path(sys.argv[2] if len(sys.argv) > 2 else ROOT / "docs" / "bench")
    out.mkdir(parents=True, exist_ok=True)
    data, sizes = load(source)
    machine = data["machine"]
    footer = f"{machine['cpu']} · panel: {machine['panel_limits']} · {machine['date'][:10]}"

    for name, getter, title, unit, fmt, lower, log in CHARTS:
        fig, ax = plt.subplots(figsize=(8, 4.4))
        bars(ax, data, sizes, getter, title, unit, fmt, lower, log)
        ax.legend(frameon=False, labelcolor=FG, loc="upper right" if not lower else "upper left")
        fig.text(0.01, 0.01, footer, color=MUTED, fontsize=7)
        fig.tight_layout(rect=(0, 0.03, 1, 1))
        fig.savefig(out / f"{name}.png", dpi=150)
        plt.close(fig)

    fig, axes = plt.subplots(2, 3, figsize=(18, 9.4))
    for ax, (name, getter, title, unit, fmt, lower, log) in zip(axes.flat, CHARTS):
        bars(ax, data, sizes, getter, title, unit, fmt, lower, log)
    handles, labels = axes.flat[0].get_legend_handles_labels()
    fig.legend(handles, labels, loc="upper center", ncol=len(labels), frameon=False, labelcolor=FG, fontsize=13)
    fig.text(0.01, 0.005, footer, color=MUTED, fontsize=8)
    fig.tight_layout(rect=(0, 0.02, 1, 0.95))
    fig.savefig(out / "overview.png", dpi=110)
    print("charts in", out)


if __name__ == "__main__":
    main()
