#!/usr/bin/env python3
"""Package the Windows center service, embedded console and linked guides."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import shutil
import struct
import subprocess
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'target' / 'center-dist')
    args = parser.parse_args()
    data = args.binary.read_bytes()
    if len(data) < 64 or data[:2] != b'MZ':
        raise SystemExit('A Windows PE executable is required.')
    offset = struct.unpack_from('<I', data, 0x3c)[0]
    if offset + 6 > len(data) or data[offset:offset+4] != b'PE\0\0' or struct.unpack_from('<H',data,offset+4)[0] != 0x8664:
        raise SystemExit('The center package requires a Windows AMD64 executable.')
    version = tomllib.loads((ROOT/'crates/center/Cargo.toml').read_text())['package']['version']
    commit = subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip()
    metadata = json.loads(subprocess.check_output(
        [str(args.binary.resolve()), '--build-info'], text=True, timeout=15
    ))
    expected = {'product':'geoledger-center','version':version,'commit':commit,
                'source_status':'clean','target':'x86_64-pc-windows-msvc',
                'profile':'release','c_runtime':'static'}
    if any(metadata.get(key) != value for key,value in expected.items()):
        raise SystemExit('Binary build metadata differs from the clean release checkout.')
    if subprocess.check_output(['git','status','--porcelain','--untracked-files=normal'],cwd=ROOT,text=True).strip():
        raise SystemExit('Package from a clean source checkout.')
    name = f'geoledger-center-v{version}-windows-x86_64'
    args.output.mkdir(parents=True,exist_ok=True)
    stage = args.output/name
    stage.mkdir(exist_ok=False)
    sources = {
        args.binary:'gl-center.exe',
        ROOT/'README.md':'PROJECT.md',
        ROOT/'docs/getting-started.md':'GETTING-STARTED.md',
        ROOT/'docs/user-guide.md':'USER-GUIDE.md',
        ROOT/'docs/api.md':'API.md',
        ROOT/'docs/development.md':'DEVELOPMENT.md',
        ROOT/'LICENSE':'LICENSE',
        ROOT/'.env.example':'config.env.example',
    }
    names = {source.resolve(): destination for source,destination in sources.items()}
    render = runpy.run_path(str(ROOT/'scripts/package-windows.py'))['render_document']
    for source,destination in sources.items():
        if source.suffix == '.md':
            (stage/destination).write_text(render(source,names,commit),encoding='utf-8')
        else:
            shutil.copyfile(source,stage/destination)
    readme = '''# GeoLedger Center

本包提供 Windows x64 中心服务 `gl-center.exe` 和内置浏览器协作测试台。

在独立开发数据库启用 PostGIS，然后在当前目录打开 PowerShell：

```powershell
$env:GL_CENTER_DATABASE_URL = Read-Host '输入 PostgreSQL 连接串'
.\\gl-center.exe tokens --out .center-tokens.json alice bob
$env:GL_CENTER_TOKEN_FILE = (Resolve-Path .center-tokens.json).Path
.\\gl-center.exe migrate
.\\gl-center.exe serve --listen 127.0.0.1:7881
```

启动后打开 `http://127.0.0.1:7881/`。不同用户分别使用自己令牌，创建私人草稿后保存与发布。
正式数据位于 `_geoledger_center.features`，属性采用 JSONB，几何采用 PostGIS geometry。

[完整启动步骤](GETTING-STARTED.md#中心版) · [协作操作](USER-GUIDE.md#中心版) · [接口](API.md#中心版) · [开发说明](DEVELOPMENT.md#中心版)

发布包的 SHA256 与 `SHA256SUMS.txt` 用于文件核验。当前二进制为 unsigned；按组织的软件执行策略运行。
本地版 `gl.exe` 独立发布，保留原来的本地仓库与业务表版本管理方式。
'''
    (stage/'README-CENTER.md').write_text(readme,encoding='utf-8')
    info={**metadata,'binary_sha256':hashlib.sha256(data).hexdigest(),'center_schema_version':1,'signature':'unsigned','features':['http','browser-console','postgis','private-workspaces','three-way-merge']}
    (stage/'build-info.json').write_text(json.dumps(info,indent=2)+'\n',encoding='utf-8')
    lines=[f'{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}' for p in sorted(stage.iterdir())]
    (stage/'SHA256SUMS.txt').write_text('\n'.join(lines)+'\n',encoding='ascii')
    archive=args.output/(name+'.zip')
    with zipfile.ZipFile(archive,'x',zipfile.ZIP_DEFLATED,compresslevel=9) as out:
        for file in sorted(stage.iterdir()): out.write(file,arcname=f'{name}/{file.name}')
    digest=hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_suffix('.zip.sha256').write_text(f'{digest}  {archive.name}\n',encoding='ascii')
    print(json.dumps({'archive':str(archive),'sha256':digest,'bytes':archive.stat().st_size}))

if __name__=='__main__': main()
