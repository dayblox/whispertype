"""Alternate native app builds to reduce time/order bias in CPU comparisons."""
import argparse
import json
import pathlib
import statistics
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--before', type=pathlib.Path, required=True)
parser.add_argument('--after', type=pathlib.Path, required=True)
parser.add_argument('--output', type=pathlib.Path, default=pathlib.Path('benchmarks'))
parser.add_argument('--seconds', type=float, default=15)
parser.add_argument('--trials', type=int, default=3)
args = parser.parse_args()
assert 5 <= args.seconds <= 60 and 2 <= args.trials <= 10
assert sys.platform == 'win32'
runner = pathlib.Path(__file__).with_name('measure-windows-efficiency.py')
reports = {'before': [], 'after': []}
with tempfile.TemporaryDirectory(prefix='whispertype-comparison-') as folder:
    for trial in range(args.trials):
        order = ['before', 'after'] if trial % 2 == 0 else ['after', 'before']
        for position, side in enumerate(order):
            output = pathlib.Path(folder) / f'{trial}-{side}.json'
            print(f'Pair {trial+1}/{args.trials}, {side}', flush=True)
            subprocess.run([sys.executable, str(runner), '--binary', str(getattr(args, side)),
                '--label', side, '--output', str(output), '--seconds', str(args.seconds),
                '--trials', '1'], check=True)
            report = json.loads(output.read_text(encoding='utf-8'))
            for sample in report['samples']:
                sample['trial'] = trial + 1
                sample['order_in_pair'] = position + 1
            reports[side].append(report)

args.output.mkdir(parents=True, exist_ok=True)
for side, series in reports.items():
    report = dict(series[0])
    report['label'] = side + ('-v0.1.4' if side == 'before' else '-v0.1.5')
    report['trials'] = args.trials
    report['methodology'] += ' Final comparison uses adjacent paired runs in alternating order (AB, BA, AB).'
    report['samples'] = [sample for item in series for sample in item['samples']]
    report['audio_trace'] = [trace for item in series for trace in item['audio_trace']]
    for state, fields in report['summary'].items():
        rows = [row for row in report['samples'] if row['state'] == state]
        for key in fields:
            if key == 'cpu_range':
                fields[key] = [min(row['cpu_percent_one_core'] for row in rows),
                               max(row['cpu_percent_one_core'] for row in rows)]
            else:
                fields[key] = statistics.median(row[key] for row in rows)
    (args.output / f'windows-efficiency-{side}.json').write_text(
        json.dumps(report, indent=2)+'\n', encoding='utf-8')
print('Saved paired before/after results to '+str(args.output), flush=True)
