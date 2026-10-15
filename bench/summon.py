#!/usr/bin/env python3
"""Summon bench: latency, memory, growth, and idle CPU of a resident GPUI process.

Runs BIN with a Unix socket path as its only argument, inside the Wayland session
named by the environment. scripts/bench.sh provides one under anvil. BIN must
accept the lines "open", "close", and "quit" on the socket, and print
`PROBE seq=<n> ev=<name> mono_ns=<CLOCK_MONOTONIC ns>` lines on stdout. It prints
app_ready with seq 0 once its socket is bound, and for summon n the events in
SUMMON_EVENTS with seq n. frame1_presented is the client's buffer attach and
commit, which is the summon time. frame1_done is the next frame GPUI draws, which
the bench waits for. GPUI throttles an unfocused window to 30 fps and anvil never
focuses it, so frame1_done is not a latency (notes/05). bench/probe is that
program until the palette exists.

Each process is measured hidden, then summoned and closed --cycles times. The
first --warm summons are timed. docs/decisions.md#budgets has the limits, and
docs/research/notes/05-probe-summon-latency.md explains each measurement."""
import argparse
import collections
import datetime
import json
import os
import re
import socket
import statistics
import subprocess
import sys
import tempfile
import threading
import time

CLK = os.sysconf("SC_CLK_TCK")
IDLE_SECS = 10.0
EVENT = re.compile(r"PROBE seq=(\d+) ev=(\S+) mono_ns=(\d+)")
SMAPS_HEADER = re.compile(r"^[0-9a-f]+-[0-9a-f]+ \S+ \S+ \S+ \S+\s*(.*)$")
SUMMON_EVENTS = ["toggle_handled_open", "first_render", "window_opened",
                 "frame1_pre_present", "frame1_presented", "frame1_done"]


def rollup(pid):
    d = {}
    for line in open(f"/proc/{pid}/smaps_rollup"):
        k, _, rest = line.partition(":")
        if rest.strip().endswith("kB"):
            d[k] = int(rest.split()[0])
    return {k: d.get(k, 0) for k in ("Rss", "Pss", "Pss_Anon", "Pss_File", "Pss_Shmem", "Private_Dirty", "Shared_Clean")}


def cpu_ticks(pid):
    """Returns (utime + stime in clock ticks, thread count)."""
    f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return int(f[11]) + int(f[12]), int(f[17])


def ctxsw(pid):
    tot = 0
    for t in os.listdir(f"/proc/{pid}/task"):
        try:
            for line in open(f"/proc/{pid}/task/{t}/status"):
                if "ctxt_switches" in line:
                    tot += int(line.split()[1])
        except FileNotFoundError:
            pass
    return tot


def breakdown(pid, top=14):
    pss = collections.Counter()
    private_dirty = collections.Counter()
    name = None
    for line in open(f"/proc/{pid}/smaps"):
        m = SMAPS_HEADER.match(line)
        if m:
            name = m.group(1).strip() or "[anon]"
            name = re.sub(r"^/memfd:.*", "[memfd]", name)
            name = re.sub(r" \(deleted\)$", "", name)
            continue
        if line.startswith("Pss:"):
            pss[name] += int(line.split()[1])
        elif line.startswith("Private_Dirty:"):
            private_dirty[name] += int(line.split()[1])
    return [(n, kb, private_dirty[n]) for n, kb in pss.most_common(top)]


def idle_cpu(pid):
    c0, _ = cpu_ticks(pid)
    s0 = ctxsw(pid)
    t0 = time.monotonic()
    time.sleep(IDLE_SECS)
    c1, nthr = cpu_ticks(pid)
    s1 = ctxsw(pid)
    dt = time.monotonic() - t0
    return dict(cpu_pct=100.0 * (c1 - c0) / CLK / dt, ctxsw_per_s=(s1 - s0) / dt, threads=nthr)


class Proc:
    def __init__(self, args, env, stderr):
        self.p = subprocess.Popen(args, env=env, stdout=subprocess.PIPE, stderr=stderr, text=True)
        self.stderr_path = stderr.name
        self.ev = collections.defaultdict(dict)  # seq -> ev -> mono_ns
        self.eof = False
        self.cv = threading.Condition()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in self.p.stdout:
            m = EVENT.match(line)
            if m:
                with self.cv:
                    self.ev[int(m.group(1))][m.group(2)] = int(m.group(3))
                    self.cv.notify_all()
        with self.cv:
            self.eof = True
            self.cv.notify_all()

    def wait_ev(self, seq, ev, timeout=15):
        with self.cv:
            self.cv.wait_for(lambda: ev in self.ev.get(seq, {}) or self.eof, timeout)
        if ev not in self.ev.get(seq, {}):
            why = f"it exited with {self.p.wait()}" if self.eof else "timed out"
            raise RuntimeError(f"no seq={seq} ev={ev}: {why}; see {self.stderr_path}")
        return self.ev[seq][ev]


def summ(xs):
    xs = sorted(xs)
    if not xs:
        return "n/a"
    return f"median {statistics.median(xs):7.1f}  min {xs[0]:7.1f}  max {xs[-1]:7.1f}  n={len(xs)}"


def slope(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    mx, my = statistics.mean(xs), statistics.mean(ys)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)


def run_proc(binary, sock, warm, cycles, env, stderr):
    pr = Proc([binary, sock], env, stderr)
    try:
        pr.wait_ev(0, "app_ready")
        time.sleep(2.0)
        pid = pr.p.pid
        res = dict(mem={}, summons=[], drift=[])
        res["mem"]["hidden_before"] = rollup(pid)
        res["breakdown_hidden_before"] = breakdown(pid)
        res["idle_before"] = idle_cpu(pid)
        c = socket.socket(socket.AF_UNIX)
        c.connect(sock)
        f = c.makefile("w")

        def send(cmd):
            t = time.monotonic_ns()
            f.write(f"{cmd}\n")
            f.flush()
            return t

        for k in range(1, cycles + 1):
            t0 = send("open")
            done = pr.wait_ev(k, "frame1_done")
            with pr.cv:
                ev = dict(pr.ev[k])
            if k <= warm:
                res["summons"].append({e: (ev[e] - t0) / 1e6 for e in SUMMON_EVENTS})
            time.sleep(max(0, done / 1e9 + 0.4 - time.monotonic()))
            if k == 1:
                res["mem"]["shown_1"] = rollup(pid)
            if k == warm:
                res["mem"][f"shown_{warm}"] = rollup(pid)
            send("close")
            time.sleep(0.4)
            r = rollup(pid)
            if k == 1:
                res["mem"]["hidden_after_1"] = r
                res["breakdown_hidden_after_1"] = breakdown(pid)
            _, threads = cpu_ticks(pid)
            res["drift"].append(dict(cycle=k, Pss=r["Pss"], Pss_Anon=r["Pss_Anon"], threads=threads))
        time.sleep(1.0)
        res["mem"][f"hidden_after_{cycles}"] = rollup(pid)
        res["breakdown_hidden_after_all"] = breakdown(pid)
        res["idle_after"] = idle_cpu(pid)
        res["thread_names"] = collections.Counter(
            open(f"/proc/{pid}/task/{t}/comm").read().strip() for t in os.listdir(f"/proc/{pid}/task")
        ).most_common(12)
        f.write("quit\n")
        f.flush()
        pr.p.wait(timeout=5)
        return res
    finally:
        if pr.p.poll() is None:
            pr.p.kill()


def report(runs, warm, cycles, stderr_path):
    print(f"== summon latency, ms after \"open\" was sent, {len(runs)} procs. "
          "frame1_presented is the summon time; frame1_done is throttled to 30 fps.")
    for title, picks in (("summon #1, the first window in the process", [r["summons"][0] for r in runs]),
                         (f"summons #2..#{warm}, pooled", [s for r in runs for s in r["summons"][1:]])):
        print(f"  {title}")
        for e in SUMMON_EVENTS:
            print(f"    {e:20s} {summ([s[e] for s in picks])}")
    print("== memory, MiB, median over procs")
    print(f"  {'state':18s} {'Pss':>7s} {'Pss_Anon':>9s} {'Private_Dirty':>14s}")
    for state in runs[0]["mem"]:
        vals = [statistics.median(r["mem"][state][k] for r in runs) / 1024 for k in ("Pss", "Pss_Anon", "Private_Dirty")]
        print(f"  {state:18s} {vals[0]:7.1f} {vals[1]:9.1f} {vals[2]:14.1f}")
    print(f"== growth over cycles {warm + 1}..{cycles}, kB per cycle, per proc")
    for key in ("Pss", "Pss_Anon"):
        print(f"  {key:9s} {[round(slope([(p['cycle'], p[key]) for p in r['drift'] if p['cycle'] > warm]), 1) for r in runs]}")
    print(f"== idle CPU while hidden, {IDLE_SECS:.0f} s, per proc")
    for k in ("idle_before", "idle_after"):
        print(f"  {k:12s} cpu% {[round(r[k]['cpu_pct'], 2) for r in runs]}  "
              f"ctxsw/s {[round(r[k]['ctxsw_per_s'], 1) for r in runs]}  threads {[r[k]['threads'] for r in runs]}")
    adapters = sorted({line.split("] ", 1)[-1].strip() for line in open(stderr_path) if "Selected GPU adapter" in line})
    print("== adapter:", "; ".join(adapters) or "not logged")


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("binary")
    ap.add_argument("--procs", type=int, default=3)
    ap.add_argument("--warm", type=int, default=20)
    ap.add_argument("--cycles", type=int, default=100)
    ap.add_argument("--out", default="target/bench")
    a = ap.parse_args()
    if a.procs < 1 or a.warm < 1:
        ap.error("--procs and --warm must be at least 1")
    if a.cycles < a.warm + 2:
        ap.error("--cycles must exceed --warm by at least 2, to fit a growth slope")
    if "WAYLAND_DISPLAY" not in os.environ:
        sys.exit("summon.py: WAYLAND_DISPLAY is not set; run it through scripts/bench.sh")
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    stderr_path = os.path.join(a.out, f"summon-{stamp}.stderr.log")
    env = {k: v for k, v in os.environ.items() if k not in ("DISPLAY", "WAYLAND_DEBUG")}
    env["RUST_LOG"] = "gpui_wgpu=info,warn"
    # XDG_RUNTIME_DIR keeps the socket path short and private.
    sockroot = os.environ.get("XDG_RUNTIME_DIR")
    with open(stderr_path, "w") as stderr, tempfile.TemporaryDirectory(prefix="summon-", dir=sockroot) as sockdir:
        runs = [run_proc(os.path.abspath(a.binary), os.path.join(sockdir, f"{i}.sock"), a.warm, a.cycles, env, stderr)
                for i in range(a.procs)]
    json_path = os.path.join(a.out, f"summon-{stamp}.json")
    json.dump(dict(args=vars(a), runs=runs), open(json_path, "w"), indent=1)
    report(runs, a.warm, a.cycles, stderr_path)
    print("== raw data:", json_path)


if __name__ == "__main__":
    main()
