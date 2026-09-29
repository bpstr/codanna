#!/usr/bin/env python3
"""Measure process-crash recovery on macOS using local sources and no model.

Keeps the interrupted/resumed index before comparing a forced rebuild. It proves
only the observed crash point, not every partial-write or incoming-edge failure.
"""
import argparse, hashlib, importlib.util, json, pathlib, subprocess, sys, time

parser=argparse.ArgumentParser(description="Offline interrupted-code-index release witness; exit 1 means qualification failed")
parser.add_argument('--codanna',type=pathlib.Path,required=True)
parser.add_argument('--out',type=pathlib.Path,required=True)
args=parser.parse_args()
if sys.version_info < (3,11):parser.error('Python 3.11 or newer is required')
if sys.platform != 'darwin':parser.error('This witness requires macOS cp -c for retained index snapshots')
repo=pathlib.Path(__file__).resolve().parents[2]
root=args.out.resolve()
if root.is_relative_to(repo):parser.error('Keep qualification output outside the checkout')
binary=args.codanna.resolve()
if not binary.is_file():parser.error('Codanna executable does not exist')
root.mkdir(parents=True,exist_ok=False)
ws,home=root/'workspace',root/'home';ws.mkdir();home.mkdir();(ws/'src').mkdir()
spec=importlib.util.spec_from_file_location('acceptance',str(pathlib.Path(__file__).with_name('run.py')));h=importlib.util.module_from_spec(spec);spec.loader.exec_module(h)
config=h.write_settings(ws,False);env=h.isolated_env(home)
base=[str(binary),'--config',str(config)]
report={'binary':str(binary),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),'scope':'Deterministic code-index process termination after observed Tantivy publication; exact internal phase is not instrumented','commands':[]}
def save():(root/'report.json').write_text(json.dumps(report,indent=2)+'\n')
def run(label,args):
    start=time.monotonic();r=subprocess.run(base+args,cwd=ws,env=env,capture_output=True,text=True,timeout=180)
    (root/(label+'.out')).write_text(r.stdout);(root/(label+'.err')).write_text(r.stderr)
    report['commands'].append({'label':label,'exit_code':r.returncode,'seconds':time.monotonic()-start});save()
    assert r.returncode==0 or (label.startswith('resumed-calls') and r.returncode==1),label
    return r.stdout
def source(number):
    text='\n'.join(f'pub fn chain_{number}_{n}() -> u32 {{ '+(f'chain_{number}_{n+1}()' if n<99 else '99')+' }' for n in range(100))+'\n'
    (ws/f'src/unit_{number}.rs').write_text(text)
for n in range(5):source(n)
run('seed',['index','src','--threads','2','--no-progress'])
for n in range(5,205):source(n)
metadata=ws/'.codanna/index/tantivy/meta.json';before=hashlib.sha256(metadata.read_bytes()).hexdigest()
with (root/'interrupted.out').open('w') as out,(root/'interrupted.err').open('w') as err:
    proc=subprocess.Popen(base+['index','src','--threads','2','--no-progress'],cwd=ws,env=env,stdout=out,stderr=err)
    try:
        deadline=time.monotonic()+120
        while proc.poll() is None and hashlib.sha256(metadata.read_bytes()).hexdigest()==before:
            assert time.monotonic()<deadline
            time.sleep(.005)
        report['publication_changed_before_kill']=hashlib.sha256(metadata.read_bytes()).hexdigest()!=before
        report['process_alive_before_kill']=proc.poll() is None;save()
        assert report['publication_changed_before_kill'] and report['process_alive_before_kill']
        (root/'interrupted-meta.json').write_bytes(metadata.read_bytes())
        proc.kill();report['signal_exit']=proc.wait(timeout=10);save();assert report['signal_exit']==-9
    finally:
        if proc.poll() is None:proc.kill()
        proc.wait(timeout=10)
run('resume',['index','src','--threads','2','--no-progress'])
report['resumed_info']=json.loads(run('resumed-info',['mcp','get_index_info','--json']))['data'];save()
subprocess.run(['cp','-cRp',str(ws/'.codanna/index'),str(root/'resumed-index')],check=True)
for number in [0,20,50,75,100,125,150,175,200]:
    run(f'resumed-calls-{number}', ['mcp','get_calls',f'chain_{number}_0','--json'])
run('force',['index','src','--force','--threads','2','--no-progress'])
for number in [0,20,50,75,100,125,150,175,200]:
    run(f'force-calls-{number}', ['mcp','get_calls',f'chain_{number}_0','--json'])

report['force_info']=json.loads(run('force-info',['mcp','get_index_info','--json']))['data'];save()
keys=['file_count','symbol_count','relationship_count']
report['count_parity']=all(report['resumed_info'][k]==report['force_info'][k] for k in keys)
report['force_expected_counts']=report['force_info']['file_count']==205 and report['force_info']['symbol_count']==20500 and report['force_info']['relationship_count']==20295
report['edge_witnesses'] = []
for number in [0,20,50,75,100,125,150,175,200]:
    resumed=json.loads((root/f'resumed-calls-{number}.out').read_text())
    forced=json.loads((root/f'force-calls-{number}.out').read_text())
    expected=f'chain_{number}_1'
    def has_edge(result):
        return result.get('status') == 'success' and any(row.get('name')==expected and row.get('file_path')==f'src/unit_{number}.rs' for row in result.get('data',[]))
    report['edge_witnesses'].append({'source':f'chain_{number}_0','expected':expected,'resumed':has_edge(resumed),'force':has_edge(forced)})
report['pass']=report['count_parity'] and report['force_expected_counts'] and all(x['resumed'] and x['force'] for x in report['edge_witnesses']);save();print(json.dumps({k:report[k] for k in ['resumed_info','force_info','pass']}))
raise SystemExit(0 if report['pass'] else 1)
