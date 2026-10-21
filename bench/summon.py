#!/usr/bin/env python3
"""Summon bench and gate: the latency, memory, growth, and idle CPU of the lyncast daemon.

  summon.py LYNCAST PROBE [--procs N] [--warm W] [--cycles C] [--out DIR] [--real]
  summon.py --judge FILE

Runs `LYNCAST daemon --trace` in the Wayland session that WAYLAND_DISPLAY names, which
scripts/bench.sh starts under anvil, and summons it as `lyncast toggle` does: one request per
connection, timed from its write. Each process is measured hidden, then summoned and closed
--cycles times. Its first --warm summons are timed, and each of them is paired with a summon of
PROBE, bench/probe, in the same cycle. The run is judged against the budgets in
docs/decisions.md#budgets, and docs/design/testing.md explains each measurement. --judge judges
a run that summon.py wrote. --real records a run on real hardware, in the niri session that
NIRI_SOCKET names, and judges nothing.

Both programs print `PROBE seq=<n> ev=<name> mono_ns=<CLOCK_MONOTONIC ns>` lines on stdout:
app_ready with seq 0 once they serve, and for summon n the events in SUMMON_EVENTS with seq n.
frame1_presented is the client's buffer attach and commit, which is the summon time. frame1_done
is the next frame GPUI draws, which the bench waits for. GPUI throttles an unfocused window to
30 fps and anvil never focuses it, so frame1_done is not a latency (notes/05). bench/probe reads
the lines open, close, and quit on one connection and replies to none. Both programs also print
the GPU adapter that GPUI picked, and a run stops unless they drew with one Vulkan adapter and
map the same Vulkan layers."""
import argparse
import collections
import datetime
import json
import os
import random
import re
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib

CLK = os.sysconf("SC_CLK_TCK")
IDLE_SECS = 10.0
EVENT = re.compile(r"PROBE seq=(\d+) ev=(\S+) mono_ns=(\d+)")
ADAPTER = "Selected GPU adapter: "
SMAPS_HEADER = re.compile(r"^[0-9a-f]+-[0-9a-f]+ \S+ \S+ \S+ \S+\s*(.*)$")
SUMMON_EVENTS = ["toggle_handled_open", "first_render", "window_opened",
                 "frame1_pre_present", "frame1_presented", "frame1_done"]
TOGGLE = b'{"request":"toggle"}\n'
QUIT = b'{"request":"activate","key":"cmd:quit","via":{"chord":"enter"},"confirmed":false,"query":""}\n'
LINE_MAX = 65536
# The variables of the GPU driver and its shader cache, which the programs keep from the caller.
# With the Mesa cache off, a warm summon took 424.5 ms (notes/05), so XDG_CACHE_HOME stays.
GPU_VARS = re.compile(r"^(VK_|MESA_|LP_)|^XDG_CACHE_HOME$")

# The limits of docs/decisions.md#budgets.
LIMIT_MS = 50.0
LIMIT_PSS_MIB = 18.0
LIMIT_ANON_MIB = 62.0
LIMIT_GROWTH_KB = 40.0
LIMIT_IDLE_PCT = 0.1
# notes/05's probe took 44.9 ms. A probe 2 ms slower than that timed a busy box, not lyncast.
BUSY_PROBE_MS = 46.9
ATTEMPTS = 3
QUIET_WAIT = 600.0


def loadavg():
    return [float(x) for x in open("/proc/loadavg").read().split()[:3]]


def cpu_times():
    """The CPU time of the whole box in clock ticks, and the part of it idle or waiting on I/O."""
    t = [int(x) for x in open("/proc/stat").readline().split()[1:9]]
    return sum(t), t[3] + t[4]


def busy_cpus(secs=1.0):
    t0, i0 = cpu_times()
    time.sleep(secs)
    t1, i1 = cpu_times()
    return os.cpu_count() * (1.0 - (i1 - i0) / (t1 - t0))


def wait_quiet():
    """Waits until fewer than a quarter of the CPUs were busy over a second, for QUIET_WAIT at
    most, and gives how long it waited and the busy CPUs it saw last."""
    start = time.monotonic()
    while True:
        busy = busy_cpus()
        waited = time.monotonic() - start
        if busy < os.cpu_count() / 4 or waited >= QUIET_WAIT:
            return dict(waited_s=round(waited, 1), busy_cpus=round(busy, 1))
        time.sleep(4.0)


def host():
    cpu = next((line.split(":", 1)[1].strip() for line in open("/proc/cpuinfo")
                if line.startswith("model name")), "unknown")
    return dict(hostname=socket.gethostname(), nproc=os.cpu_count(), cpu=cpu, kernel=os.uname().release)


def rollup(pid):
    d = {}
    for line in open(f"/proc/{pid}/smaps_rollup"):
        k, _, rest = line.partition(":")
        if rest.strip().endswith("kB"):
            d[k] = int(rest.split()[0])
    return {k: d.get(k, 0) for k in ("Rss", "Pss", "Pss_Anon", "Pss_File", "Pss_Shmem", "Private_Dirty", "Shared_Clean")}


def thread_count(pid):
    return len(os.listdir(f"/proc/{pid}/task"))


def cpu_ns(pid):
    """The CPU time in ns of every thread that pid has run, those that ended too: the kernel's
    CPUCLOCK_SCHED clock of pid, which clock_getcpuclockid(3) gives and Python's time module does
    not wrap."""
    return time.clock_gettime_ns(((~pid) << 3) | 2)


def cpu_ticks(pid):
    f = open(f"/proc/{pid}/stat").read().rsplit(")", 1)[1].split()
    return int(f[11]) + int(f[12])


def idle_cpu(pid):
    """The CPU share of pid over IDLE_SECS by its CPU clock, and by ticks, which resolve 1/CLK s."""
    n0, c0, t0 = cpu_ns(pid), cpu_ticks(pid), time.monotonic()
    time.sleep(IDLE_SECS)
    n1, c1, dt = cpu_ns(pid), cpu_ticks(pid), time.monotonic() - t0
    if n1 < n0:
        raise RuntimeError(f"the CPU clock of {pid} went back by {n0 - n1} ns")
    return dict(sched_pct=100.0 * (n1 - n0) / 1e9 / dt, ticks=c1 - c0, tick_pct=100.0 * (c1 - c0) / CLK / dt,
                threads=thread_count(pid))


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


def png(rgb, size=48):
    raw = b"".join(b"\x00" + bytes(rgb) * size for _ in range(size))

    def chunk(kind, data):
        c = kind + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def write_fixture(share, n=300):
    """n desktop apps with a 48x48 PNG icon each under share, the same every run. Every 10th app
    has two desktop actions, and each runs `true`."""
    words = ("alpha atlas beacon cedar cinder comet delta ember falcon fern forge garnet glacier "
             "harbor helix indigo iris jasper juniper kestrel lattice lumen maple meridian nimbus "
             "nova onyx orbit pebble prism quartz quill raven ripple saffron sable sierra spruce "
             "tango tundra umber vector violet willow xenon yarrow zephyr").split()
    kinds = ("Editor Viewer Player Browser Terminal Manager Monitor Calculator Studio Recorder "
             "Mail Chat Notes Reader Drawing Scanner Backup Clock Weather Maps").split()
    cats = "Development Office Graphics AudioVideo Network System Utility Game Education".split()
    rnd = random.Random(20261017)
    apps = os.path.join(share, "applications")
    icons = os.path.join(share, "icons", "hicolor", "48x48", "apps")
    os.makedirs(apps)
    os.makedirs(icons)
    seen = set()
    for i in range(n):
        while True:
            name = f"{rnd.choice(words).title()} {rnd.choice(kinds)}"
            if rnd.random() < 0.3:
                name = f"{rnd.choice(words).title()} {name}"
            if name not in seen:
                seen.add(name)
                break
        kind = name.split()[-1]
        lines = ["[Desktop Entry]", "Type=Application", f"Name={name}", f"GenericName={kind}",
                 f"Comment=A fixture {kind.lower()} for the lyncast bench, number {i}",
                 f"Keywords={';'.join(rnd.sample(words, 3))};", f"Categories={rnd.choice(cats)};",
                 "Exec=true %U", f"Icon=fixture-{i:03d}", "Terminal=false"]
        if i % 10 == 0:
            lines += ["Actions=new-window;private;", "", "[Desktop Action new-window]",
                      "Name=New Window", "Exec=true --new-window", "",
                      "[Desktop Action private]", "Name=New Private Window", "Exec=true --private"]
        with open(os.path.join(apps, f"org.fixture.{name.lower().replace(' ', '-')}.desktop"), "w") as f:
            f.write("\n".join(lines) + "\n")
        with open(os.path.join(icons, f"fixture-{i:03d}.png"), "wb") as f:
            f.write(png((rnd.randrange(256), rnd.randrange(256), rnd.randrange(256))))


class Proc:
    """A running program and the PROBE events it has printed, by summon and event."""

    def __init__(self, args, env, stderr):
        self.p = subprocess.Popen(args, env=env, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                  stderr=stderr, text=True)
        self.stderr_path = stderr.name
        self.ev = collections.defaultdict(dict)
        self.adapter = None
        self.eof = False
        self.cv = threading.Condition()
        threading.Thread(target=self._read, daemon=True).start()

    def _read(self):
        for line in iter(lambda: self.p.stdout.readline(LINE_MAX), ""):
            m = EVENT.match(line)
            with self.cv:
                if m:
                    self.ev[int(m.group(1))][m.group(2)] = int(m.group(3))
                elif ADAPTER in line and self.adapter is None:
                    self.adapter = line.split(ADAPTER, 1)[1].strip()
                self.cv.notify_all()
        with self.cv:
            self.eof = True
            self.cv.notify_all()

    def wait_ev(self, seq, ev, timeout=15):
        with self.cv:
            self.cv.wait_for(lambda: ev in self.ev.get(seq, {}) or self.eof, timeout)
            if ev in self.ev.get(seq, {}):
                return self.ev[seq][ev]
        why = f"it exited with {self.p.wait()}" if self.eof else "timed out"
        raise RuntimeError(f"{self.p.args[0]}: no seq={seq} ev={ev}: {why}; see {self.stderr_path}")

    def events(self, seq):
        with self.cv:
            return dict(self.ev[seq])

    def exit(self, timeout):
        try:
            return self.p.wait(timeout=timeout)
        finally:
            self.kill()

    def kill(self):
        if self.p.poll() is None:
            self.p.kill()
            self.p.wait()


class Lyncast:
    """The daemon, which reads one request per connection and replies to each."""

    def __init__(self, binary, env, stderr):
        self.sock = os.path.join(env["XDG_RUNTIME_DIR"], "lyncast.sock")
        self.proc = Proc([binary, "daemon", "--trace"], env, stderr)

    def start(self):
        self.proc.wait_ev(0, "app_ready", timeout=30)

    def request(self, line, want):
        """Sends one request and gives the time of its write. The reply must be `want`."""
        with socket.socket(socket.AF_UNIX) as c:
            c.settimeout(15)
            c.connect(self.sock)
            t = time.monotonic_ns()
            c.sendall(line)
            reply = c.makefile("rb").readline(LINE_MAX)
        if reply.strip() != want:
            raise RuntimeError(f"lyncast replied {reply!r} to {line!r}, not {want!r}")
        return t

    def open(self, k):
        return self.request(TOGGLE, b'{"reply":"summon","state":"open"}')

    def close(self, k):
        self.request(TOGGLE, b'{"reply":"summon","state":"closed"}')
        self.proc.wait_ev(k, "window_closed")

    def quit(self):
        self.request(QUIT, b'{"reply":"done","message":"ran cmd:quit"}')
        code = self.proc.exit(timeout=10)
        if code != 0:
            raise RuntimeError(f"lyncast exited with {code} after cmd:quit; see {self.proc.stderr_path}")
        return code


class Probe:
    """bench/probe, which reads lines on one connection and replies to none."""

    def __init__(self, binary, env, stderr):
        self.sock = os.path.join(env["XDG_RUNTIME_DIR"], "probe.sock")
        self.proc = Proc([binary, self.sock], env, stderr)
        self.conn = None

    def start(self):
        self.proc.wait_ev(0, "app_ready", timeout=30)
        self.conn = socket.socket(socket.AF_UNIX)
        self.conn.connect(self.sock)

    def send(self, line):
        t = time.monotonic_ns()
        self.conn.sendall(line + b"\n")
        return t

    def open(self, k):
        return self.send(b"open")

    def close(self, k):
        self.send(b"close")

    def quit(self):
        self.send(b"quit")
        self.conn.close()
        return self.proc.exit(timeout=5)


def summon(target, k):
    """Opens summon k, waits until 0.4 s after its next frame, and gives each event in ms after
    the open was sent."""
    t0 = target.open(k)
    done = target.proc.wait_ev(k, "frame1_done")
    ev = target.proc.events(k)
    time.sleep(max(0.0, done / 1e9 + 0.4 - time.monotonic()))
    return {e: (ev[e] - t0) / 1e6 for e in SUMMON_EVENTS}


def dismiss(target, k):
    target.close(k)
    time.sleep(0.4)


def vulkan_paths(env):
    """VK_DRIVER_FILES and VK_IMPLICIT_LAYER_PATH, each unless env sets it, naming the directories
    in which env's Vulkan loader looks for drivers and implicit layers, in its order. Each replaces
    the loader's search, so the daemon's own XDG directories hide neither from it."""
    home = env.get("HOME", "")
    roots = [env.get("XDG_CONFIG_HOME") or f"{home}/.config", *(env.get("XDG_CONFIG_DIRS") or "/etc/xdg").split(":"),
             "/etc", env.get("XDG_DATA_HOME") or f"{home}/.local/share",
             *(env.get("XDG_DATA_DIRS") or "/usr/local/share:/usr/share").split(":")]
    paths = {}
    for var, kind, set_by in (("VK_DRIVER_FILES", "icd.d", ("VK_DRIVER_FILES", "VK_ICD_FILENAMES")),
                              ("VK_IMPLICIT_LAYER_PATH", "implicit_layer.d", ("VK_IMPLICIT_LAYER_PATH",))):
        if not any(env.get(v) for v in set_by):
            paths[var] = ":".join(d for d in (os.path.join(r, "vulkan", kind) for r in roots) if os.path.isdir(d))
    return paths


def vulkan(proc):
    """The adapter that GPUI picked in proc, and the Vulkan layers it maps, whose libraries the
    Khronos convention names libVkLayer_*."""
    return dict(adapter=proc.adapter,
                layers=sorted({line.split()[-1] for line in open(f"/proc/{proc.p.pid}/maps") if "/libVkLayer" in line}))


def one_vulkan(ly, probe):
    """Both programs' adapters and layers. GPUI draws with GL when the loader finds no Vulkan
    driver, so the run stops unless both drew with one Vulkan adapter and map the same layers."""
    got = dict(lyncast=vulkan(ly), probe=vulkan(probe))
    if got["lyncast"] != got["probe"] or not (got["lyncast"]["adapter"] or "").endswith("(Vulkan)"):
        raise RuntimeError(f"the programs must draw with one Vulkan adapter and map the same layers: {got}")
    return got


def lyncast_env(base, home, share):
    """The daemon's variables: its own runtime, data, config, and state directories, and the
    fixture apps."""
    dirs = {v: os.path.join(home, d) for v, d in (("XDG_RUNTIME_DIR", "run"), ("XDG_DATA_HOME", "data"),
                                                   ("XDG_CONFIG_HOME", "config"), ("XDG_STATE_HOME", "state"))}
    for d in dirs.values():
        os.makedirs(d, mode=0o700)
    return dict(base, XDG_DATA_DIRS=share, **dirs)


def probe_env(base, home):
    run = os.path.join(home, "run")
    os.makedirs(run, mode=0o700)
    return dict(base, XDG_RUNTIME_DIR=run)


def warm_up(a, base, root, share, stderr):
    """Two summons of each program, which leave the Mesa shader cache warm, and their times. A
    cold cache took the first summon past 500 ms, and Pss_Anon 12 MiB higher."""
    times = {}
    for name, start in (
            ("lyncast", lambda: Lyncast(a.lyncast, lyncast_env(base, os.path.join(root, "warm-lyncast"), share), stderr)),
            ("probe", lambda: Probe(a.probe, probe_env(base, os.path.join(root, "warm-probe")), stderr))):
        target = start()
        try:
            target.start()
            times[name] = []
            for k in (1, 2):
                times[name].append(round(summon(target, k)["frame1_presented"], 1))
                dismiss(target, k)
            target.quit()
        finally:
            target.proc.kill()
    return times


def run_proc(a, base, home, share, stderr):
    """Measures one daemon. The probe runs beside it for the timed summons and quits after them."""
    res = dict(mem={}, summons=[], probe_summons=[], drift=[], loadavg_start=loadavg())
    ly = Lyncast(a.lyncast, lyncast_env(base, os.path.join(home, "lyncast"), share), stderr)
    try:
        ly.start()
        time.sleep(2.0)
        pid = ly.proc.p.pid
        res["mem"]["hidden_before"] = rollup(pid)
        res["breakdown_hidden_before"] = breakdown(pid)
        res["idle_before"] = idle_cpu(pid)
        res["quiet"] = wait_quiet()
        probe = Probe(a.probe, probe_env(base, os.path.join(home, "probe")), stderr)
        try:
            probe.start()
            for k in range(1, a.cycles + 1):
                timed = summon(ly, k)
                if k == 1:
                    res["mem"]["shown_1"] = rollup(pid)
                if k == a.warm:
                    res["mem"][f"shown_{a.warm}"] = rollup(pid)
                dismiss(ly, k)
                r = rollup(pid)
                if k == 1:
                    res["mem"]["hidden_after_1"] = r
                    res["breakdown_hidden_after_1"] = breakdown(pid)
                res["drift"].append(dict(cycle=k, Pss=r["Pss"], Pss_Anon=r["Pss_Anon"], threads=thread_count(pid)))
                if k <= a.warm:
                    res["summons"].append(timed)
                    res["probe_summons"].append(summon(probe, k))
                    dismiss(probe, k)
                if k == 1:
                    res["vulkan"] = one_vulkan(ly.proc, probe.proc)
                if k == a.warm:
                    res["probe_exit"] = probe.quit()
        finally:
            probe.proc.kill()
        time.sleep(1.0)
        res["mem"][f"hidden_after_{a.cycles}"] = rollup(pid)
        res["breakdown_hidden_after_all"] = breakdown(pid)
        res["idle_after"] = idle_cpu(pid)
        res["thread_names"] = collections.Counter(
            open(f"/proc/{pid}/task/{t}/comm").read().strip() for t in os.listdir(f"/proc/{pid}/task")
        ).most_common(12)
        res["exit"] = ly.quit()
        res["loadavg_end"] = loadavg()
        return res
    finally:
        ly.proc.kill()


def warm_times(summons):
    """frame1_presented of summons #2 and on, since #1 opens the first window of the process."""
    return [s["frame1_presented"] for s in summons[1:]]


def medians(runs):
    """The median warm summon of the daemon and of the probe, pooled over runs."""
    return (statistics.median(x for r in runs for x in warm_times(r["summons"])),
            statistics.median(x for r in runs for x in warm_times(r["probe_summons"])))


def busy(ms, probe):
    """Summons that missed the budget while the probe's beside them were slow too timed a busy box."""
    return ms > LIMIT_MS and probe > BUSY_PROBE_MS


def slope(pts):
    xs = [p[0] for p in pts]
    ys = [p[1] for p in pts]
    mx, my = statistics.mean(xs), statistics.mean(ys)
    return sum((x - mx) * (y - my) for x, y in zip(xs, ys)) / sum((x - mx) ** 2 for x in xs)


def judge(data):
    """Each budget as (passed, line), from the runs of a bench."""
    runs, w, n = data["runs"], data["args"]["warm"], data["args"]["cycles"]
    ms, probe = medians(runs)
    slow = ", a busy box" if busy(ms, probe) else ""
    pss = statistics.median(r["mem"]["hidden_before"]["Pss"] for r in runs) / 1024
    anon = statistics.median(max(r["mem"][f"shown_{w}"]["Pss_Anon"], r["mem"][f"hidden_after_{n}"]["Pss_Anon"])
                             for r in runs) / 1024
    slopes = [slope([(p["cycle"], p["Pss_Anon"]) for p in r["drift"] if p["cycle"] > w]) for r in runs]
    threads = [(r["drift"][w]["threads"], r["drift"][-1]["threads"]) for r in runs]
    grew = [f"{a} to {b}" for a, b in threads if b > a]
    idle = max(max(r["idle_before"]["sched_pct"], r["idle_after"]["sched_pct"]) for r in runs)
    return [
        (ms <= LIMIT_MS, f"warm summon: {ms:.1f} ms, probe {probe:.1f} ms{slow} ({LIMIT_MS:g} ms or less)"),
        (pss <= LIMIT_PSS_MIB, f"PSS before the first summon: {pss:.1f} MiB ({LIMIT_PSS_MIB:g} MiB or less)"),
        (anon <= LIMIT_ANON_MIB, f"Pss_Anon after summons: {anon:.1f} MiB ({LIMIT_ANON_MIB:g} MiB or less)"),
        (max(slopes) <= LIMIT_GROWTH_KB and not grew,
         f"growth over cycles {w + 1} to {n}: {max(slopes):.1f} kB per cycle at most, "
         f"{'threads ' + ', '.join(grew) if grew else 'no new threads'} ({LIMIT_GROWTH_KB:g} kB per cycle or less)"),
        (idle < LIMIT_IDLE_PCT, f"idle CPU: {idle:.3f}% at most (under {LIMIT_IDLE_PCT:g}%)"),
    ]


def print_verdicts(data):
    """Prints each budget's verdict and gives the exit status, 1 when any budget failed."""
    verdicts = judge(data)
    for passed, line in verdicts:
        print(f"{'pass' if passed else 'fail'} {line}")
    failed = sum(1 for passed, _ in verdicts if not passed)
    print(f"summon.py: {failed} of {len(verdicts)} budgets failed" if failed else "summon.py: every budget passed")
    return 1 if failed else 0


def summ(xs):
    xs = sorted(xs)
    return f"median {statistics.median(xs):7.1f}  min {xs[0]:7.1f}  max {xs[-1]:7.1f}  n={len(xs)}"


def report(data):
    runs, w, n = data["runs"], data["args"]["warm"], data["args"]["cycles"]
    h = data["host"]
    print(f"== {h['hostname']}, {h['nproc']} CPUs, {h['cpu']}, Linux {h['kernel']}")
    print("== adapter of both programs:", "; ".join(sorted({r["vulkan"]["lyncast"]["adapter"] for r in runs})))
    print("== Vulkan layers of both programs:",
          "; ".join(sorted({", ".join(r["vulkan"]["lyncast"]["layers"]) or "none" for r in runs})))
    print(f"== load average at the start and end: {data['loadavg_start']} {data['loadavg_end']}; per process "
          f"{[(r['loadavg_start'][0], r['loadavg_end'][0]) for r in runs]}")
    print(f"== waits for a quiet box before the timed summons: {[r['quiet'] for r in runs]}; "
          f"runs again after a busy box: {len(data['busy'])}")
    print(f"== warm-up, frame1_presented of summons #1 and #2, ms: {data['warmup']}")
    print(f"== summon latency, ms after the request was written, {len(runs)} procs")
    for who, key in (("lyncast", "summons"), ("probe", "probe_summons")):
        for title, picks in (("#1", [r[key][0] for r in runs]), (f"#2..#{w}", [s for r in runs for s in r[key][1:]])):
            print(f"  {who} {title}")
            for e in SUMMON_EVENTS:
                print(f"    {e:20s} {summ([s[e] for s in picks])}")
    print("== memory, MiB, median over procs")
    print(f"  {'state':18s} {'Pss':>7s} {'Pss_Anon':>9s} {'Private_Dirty':>14s}")
    for state in runs[0]["mem"]:
        vals = [statistics.median(r["mem"][state][k] for r in runs) / 1024 for k in ("Pss", "Pss_Anon", "Private_Dirty")]
        print(f"  {state:18s} {vals[0]:7.1f} {vals[1]:9.1f} {vals[2]:14.1f}")
    print(f"== growth over cycles {w + 1}..{n}, kB per cycle, per proc")
    for key in ("Pss", "Pss_Anon"):
        print(f"  {key:9s} {[round(slope([(p['cycle'], p[key]) for p in r['drift'] if p['cycle'] > w]), 1) for r in runs]}")
    print(f"  threads at cycles {w + 1} and {n}: {[(r['drift'][w]['threads'], r['drift'][-1]['threads']) for r in runs]}")
    print(f"== idle CPU while hidden, {IDLE_SECS:.0f} s, per proc")
    for k in ("idle_before", "idle_after"):
        print(f"  {k:12s} CPU clock % {[round(r[k]['sched_pct'], 4) for r in runs]}  ticks {[r[k]['ticks'] for r in runs]}  "
              f"threads {[r[k]['threads'] for r in runs]}")


def bench(a):
    if a.real and not os.environ.get("NIRI_SOCKET"):
        print("summon.py: --real runs inside niri, but NIRI_SOCKET is not set", file=sys.stderr)
        return 2
    for name in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR"):
        if not os.environ.get(name):
            sys.exit(f"summon.py: {name} is not set; run it through scripts/bench.sh")
    runtime = os.environ["XDG_RUNTIME_DIR"]
    base = {k: v for k, v in os.environ.items()
            if k in ("PATH", "HOME") or GPU_VARS.match(k)
            or (a.real and k in ("NIRI_SOCKET", "DBUS_SESSION_BUS_ADDRESS"))}
    # An absolute WAYLAND_DISPLAY lets each program have a runtime directory of its own.
    base["WAYLAND_DISPLAY"] = os.path.join(runtime, os.environ["WAYLAND_DISPLAY"])
    base["XDG_CURRENT_DESKTOP"] = "niri"
    base.update(vulkan_paths(os.environ))
    os.makedirs(a.out, exist_ok=True)
    stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    stem = os.path.join(a.out, f"summon-{'real-' if a.real else ''}{stamp}")
    args = dict(lyncast=os.path.abspath(a.lyncast), probe=os.path.abspath(a.probe), procs=a.procs,
                warm=a.warm, cycles=a.cycles, real=a.real)
    a.lyncast, a.probe = args["lyncast"], args["probe"]
    data = dict(host=host(), args=args, loadavg_start=loadavg(), runs=[], busy=[])
    # The runtime directories go under XDG_RUNTIME_DIR, so each socket path fits in 108 bytes.
    with open(f"{stem}.stderr.log", "w") as stderr, tempfile.TemporaryDirectory(prefix="bench-", dir=runtime) as root:
        share = os.path.join(root, "share")
        write_fixture(share)
        data["warmup"] = warm_up(a, base, root, share, stderr)
        for i in range(a.procs):
            for attempt in range(ATTEMPTS):
                run = run_proc(a, base, os.path.join(root, f"{i}.{attempt}"), share, stderr)
                ms, probe = medians([run])
                if a.real or attempt == ATTEMPTS - 1 or not busy(ms, probe):
                    break
                data["busy"].append(dict(proc=i, ms=ms, probe_ms=probe))
            data["runs"].append(run)
    data["loadavg_end"] = loadavg()
    with open(f"{stem}.json", "w") as f:
        json.dump(data, f, indent=1)
    report(data)
    print("== raw data:", f"{stem}.json")
    if a.real:
        print("summon.py: real hardware, so no budget is judged")
        return 0
    return print_verdicts(data)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("lyncast", nargs="?")
    ap.add_argument("probe", nargs="?")
    ap.add_argument("--procs", type=int, default=3)
    ap.add_argument("--warm", type=int, default=20)
    ap.add_argument("--cycles", type=int, default=100)
    ap.add_argument("--out", default="target/bench")
    ap.add_argument("--real", action="store_true", help="record a run on real hardware and judge nothing")
    ap.add_argument("--judge", metavar="FILE", help="judge a run that summon.py wrote")
    a = ap.parse_args()
    if a.judge:
        with open(a.judge) as f:
            return print_verdicts(json.load(f))
    if not (a.lyncast and a.probe):
        ap.error("LYNCAST and PROBE are required")
    if a.procs < 1 or a.warm < 2:
        ap.error("--procs must be at least 1 and --warm at least 2")
    if a.cycles < a.warm + 2:
        ap.error("--cycles must exceed --warm by at least 2, to fit a growth slope")
    return bench(a)


if __name__ == "__main__":
    sys.exit(main())
