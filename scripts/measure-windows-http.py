"""Paired, app-only HTTP helper profiles. Invalid key; synthetic silence only.

Runs real Schannel/ureq HTTPS requests, so latency is not a controlled proxy
benchmark. Never reads the user's key/profile, changes the proxy, or transcribes
real audio. Uses poison environment proxies to check Windows ignores them.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import hashlib
import json
import os
import pathlib
import statistics
import subprocess
import tempfile
import time
import wave
import io

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--before', type=pathlib.Path, required=True)
parser.add_argument('--after', type=pathlib.Path, required=True)
parser.add_argument('--output', type=pathlib.Path, required=True)
args = parser.parse_args()
kernel = c.WinDLL('kernel32', use_last_error=True)
psapi = c.WinDLL('psapi', use_last_error=True)
kernel.GetProcessTimes.argtypes = [w.HANDLE] + [c.POINTER(w.FILETIME)] * 4
kernel.QueryProcessCycleTime.argtypes = [w.HANDLE, c.POINTER(c.c_ulonglong)]
kernel.GetProcessHandleCount.argtypes = [w.HANDLE, c.POINTER(w.DWORD)]

class Memory(c.Structure):
    _fields_ = [('cb', w.DWORD), ('faults', w.DWORD)] + [(name, c.c_size_t) for name in (
        'peak_working_set', 'working_set', 'peak_paged', 'paged', 'peak_nonpaged',
        'nonpaged', 'pagefile', 'peak_pagefile', 'private_bytes')]

psapi.GetProcessMemoryInfo.argtypes = [w.HANDLE, c.POINTER(Memory), w.DWORD]

def snapshot(process):
    handle = w.HANDLE(int(process._handle))
    times = [w.FILETIME() for _ in range(4)]
    assert kernel.GetProcessTimes(handle, *[c.byref(t) for t in times])
    cycles = c.c_ulonglong()
    assert kernel.QueryProcessCycleTime(handle, c.byref(cycles))
    handles = w.DWORD()
    assert kernel.GetProcessHandleCount(handle, c.byref(handles))
    memory = Memory()
    memory.cb = c.sizeof(memory)
    assert psapi.GetProcessMemoryInfo(handle, c.byref(memory), memory.cb)
    return dict(cpu_seconds=sum((t.dwHighDateTime << 32) | t.dwLowDateTime for t in times[2:]) / 1e7,
                cpu_cycles=cycles.value, private_bytes=memory.private_bytes,
                working_set_bytes=memory.working_set, peak_working_set_bytes=memory.peak_working_set,
                handles=handles.value)

wav = io.BytesIO()
with wave.open(wav, 'wb') as audio:
    audio.setnchannels(1)
    audio.setsampwidth(2)
    audio.setframerate(16000)
    audio.writeframes(bytes(32000))
samples = []
for trial in range(3):
    for label, binary in [('before', args.before.resolve()), ('after', args.after.resolve())]:
        for state in ['cold_upload', 'recording_warm', 'release_check']:
            if label == 'before' and state == 'recording_warm':
                continue  # No helper exists while recording in the baseline.
            with tempfile.TemporaryDirectory(prefix='whispertype-http-') as folder:
                env = dict(os.environ, WHISPERTYPE_HOME=folder,
                    GROQ_API_KEY='gsk_invalidtestkey000000000000000000000',
                    HTTP_PROXY='http://127.0.0.1:1', HTTPS_PROXY='http://127.0.0.1:1')
                command = ['_check-updates'] if state == 'release_check' else ['_worker']
                if state == 'recording_warm':
                    command += ['--warm']
                start = time.perf_counter()
                process = subprocess.Popen([str(binary), *command], stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env,
                    creationflags=subprocess.CREATE_NO_WINDOW)
                measurements = []
                try:
                    if state == 'recording_warm':
                        time.sleep(1)
                        warm = snapshot(process)
                        # Close without audio: tests warm-only resource lifetime.
                        process.stdin.close()
                    else:
                        process.stdin.write(wav.getvalue() if state == 'cold_upload' else b'')
                        process.stdin.close()
                    while process.poll() is None:
                        measurements.append(snapshot(process))
                        assert time.perf_counter() - start < 95, 'Helper failed to exit'
                        time.sleep(.002)
                    elapsed = time.perf_counter() - start
                    final = snapshot(process)
                    reply = json.loads(process.stdout.read())
                    error = reply.get('error') or ''
                    if state == 'cold_upload':
                        assert 'API key or model permission' in error, error
                    elif state == 'recording_warm':
                        assert 'Missing worker WAV header' in error, error
                    else:
                        assert not error, error
                        assert not reply.get('staged'), 'Unexpected newer release; inspect temporary profile'
                    row = dict(trial=trial+1, label=label, state=state, elapsed_seconds=elapsed,
                        cpu_seconds=final['cpu_seconds'], cpu_cycles=final['cpu_cycles'],
                        peak_working_set_bytes=final['peak_working_set_bytes'],
                        peak_private_bytes=max((r['private_bytes'] for r in measurements), default=0),
                        reply_error=error)
                    if state == 'recording_warm':
                        row['warm_snapshot'] = warm
                    samples.append(row)
                    print(json.dumps(row), flush=True)
                finally:
                    if process.poll() is None:
                        process.kill()
                    process.wait(timeout=5)

summary = {}
for label in ['before', 'after']:
    summary[label] = {}
    for state in ['cold_upload', 'recording_warm', 'release_check']:
        rows = [r for r in samples if r['label'] == label and r['state'] == state]
        if rows:
            summary[label][state] = {field: statistics.median(r[field] for r in rows) for field in (
                'elapsed_seconds', 'cpu_seconds', 'cpu_cycles', 'peak_working_set_bytes', 'peak_private_bytes')}
report = dict(methodology=__doc__, trials=3, summary=summary, samples=samples,
    binaries={label: dict(bytes=b.stat().st_size, sha256=hashlib.sha256(b.read_bytes()).hexdigest())
              for label, b in [('before', args.before), ('after', args.after)]})
args.output.write_text(json.dumps(report, indent=2)+'\n', encoding='utf-8')
