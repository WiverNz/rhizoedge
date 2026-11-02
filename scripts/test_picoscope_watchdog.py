from pathlib import Path
import json,subprocess,tempfile,time
root=Path(__file__).resolve().parent.parent
base=root/'local/evidence/picoscope'
base.mkdir(parents=True,exist_ok=True)
folder=Path(tempfile.mkdtemp(prefix='watchdog-software-test-',dir=base))
s=(root/'scripts/picoscope-windows.ps1').read_text()
pos=s.index("$result=[ordered]@{schema_version")
# Test a copy of the exact supervisor with a sleeping worker; no USB imports.
(folder/'supervisor-test.ps1').write_text(s[:pos]+"Start-Sleep -Seconds 30\n",newline='\n')
def wp(p):return subprocess.check_output(['wslpath','-w',str(p)],text=True).strip()
r={'result_file':wp(folder/'result.json'),'output_dir':wp(folder),'process_timeout_s':1}
(folder/'request.json').write_text(json.dumps(r))
start=time.monotonic()
p=subprocess.run(['powershell.exe','-NoProfile','-ExecutionPolicy','Bypass','-File',wp(folder/'supervisor-test.ps1'),'-RequestFile',wp(folder/'request.json')],capture_output=True,text=True,timeout=10)
result=json.loads(p.stdout)
assert p.returncode==21 and result['exit_code']==21 and result['worker_exited'] is True, (p.returncode,result)
assert time.monotonic()-start<8
print('Native supervisor: timeout -> exit 21, worker terminated; no device API called.')
