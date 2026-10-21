"""playground 面板 HTTP 冒烟（需求 F2/F4 验收）。

起一块 GLASS_DEFAULT_BLOCK 玻璃 + 面板，验证：
  1. WindowConfig.movable 默认 true，且 POST /api/window {"action":"movable"} 双向生效；
  2. ResizeFeedbackOptions 默认 borderWidth=5 / alpha=1 能从 GET /api/state 读到；
  3. POST /api/resize 的 applyToAll 链路能把 borderWidth 下发到已存在的窗。
退出码 0 = 全过。
"""
import io, json, os, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
EXE = os.path.join(ROOT, 'target', 'release', 'liquid-verre-playground.exe')
PORT = 8801


def api(method, path, body=None):
    data = json.dumps(body).encode('utf-8') if body is not None else None
    req = urllib.request.Request('http://127.0.0.1:%d%s' % (PORT, path), data=data,
                                 headers={'Content-Type': 'application/json'}, method=method)
    with urllib.request.urlopen(req, timeout=5) as r:
        return json.loads(r.read().decode('utf-8'))


def main():
    env = dict(os.environ)
    for k in [k for k in env if k.startswith('GLASS_')]:
        del env[k]
    env.update({'GLASS_DEFAULT_BLOCK': '1', 'GLASS_MONITORS': '1', 'GLASS_SYNTH_CAP': '1',
                'GLASS_ADAPTIVE': '0', 'GLASS_ENV_EMA': '0'})
    p = subprocess.Popen([EXE, '--no-browser', '--port', str(PORT)], env=env, cwd=ROOT,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                         creationflags=0x08000000)
    fails = []
    try:
        for _ in range(60):
            time.sleep(0.5)
            try:
                st = api('GET', '/api/state')
                break
            except OSError:
                continue
        else:
            print('FAIL 面板未启动')
            return 1
        wins = []
        for _ in range(40):
            wins = api('GET', '/api/state')['windows']
            if wins:
                break
            time.sleep(0.5)
        assert wins, 'GLASS_DEFAULT_BLOCK=1 应有一块窗'
        wid = wins[0]['id']
        assert wins[0]['movable'] is True, '默认 movable=true'
        print('ok 默认 movable=true')
        api('POST', '/api/window', {'action': 'movable', 'id': wid, 'value': False})
        time.sleep(0.4)
        w = [x for x in api('GET', '/api/state')['windows'] if x['id'] == wid][0]
        if w['movable'] is not False:
            fails.append('movable=false 未生效: %r' % w['movable'])
        else:
            print('ok POST movable=false 生效')
        api('POST', '/api/window', {'action': 'movable', 'id': wid, 'value': True})
        time.sleep(0.4)
        w = [x for x in api('GET', '/api/state')['windows'] if x['id'] == wid][0]
        if w['movable'] is not True:
            fails.append('movable=true 未恢复: %r' % w['movable'])
        else:
            print('ok POST movable=true 恢复')
        # border 默认值应能从面板读到 5 / 1.0
        rz = w['resize']['feedback']
        if abs(rz['borderWidth'] - 5.0) > 1e-6 or abs(rz['alpha'] - 1.0) > 1e-6:
            fails.append('feedback 默认值异常: %r' % rz)
        else:
            print('ok 面板读到 feedback borderWidth=5 alpha=1')
        api('POST', '/api/resize', {'options': {'feedback': {'borderWidth': 20}}, 'applyToAll': True})
        time.sleep(0.4)
        w = [x for x in api('GET', '/api/state')['windows'] if x['id'] == wid][0]
        got = w['resize']['feedback']['borderWidth']
        if abs(got - 20.0) > 1e-6:
            fails.append('POST borderWidth=20 未生效: %r' % got)
        else:
            print('ok POST borderWidth=20 生效（applyToAll 链路通）')
    finally:
        p.terminate()
        try:
            p.wait(5)
        except subprocess.TimeoutExpired:
            p.kill()
    for f in fails:
        print('FAIL', f)
    print('SMOKE', 'PASS' if not fails else 'FAIL')
    return 1 if fails else 0


if __name__ == '__main__':
    sys.exit(main())
