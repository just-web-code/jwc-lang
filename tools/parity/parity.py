"""Drive `jwc serve` and the `jwc build` binary of one application through
the same scenario, each on its own fresh database, and diff every response:
status, content type, body and redirect target. Called by `run.sh`.

    python3 parity.py <scenario> <app-dir> <serve-base> <native-base>

What differs legitimately between two runs is normalised away first —
timestamps, JWTs, page cursors, and values the application generates at
random (a short link's code). Everything else has to be identical.
"""
import json, re, sys, time, urllib.request, urllib.error

TS = re.compile(r'\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(\.\d+)?(Z|[+-]\d{2}:\d{2})')
JWT = re.compile(r'eyJ[\w-]+\.[\w-]+\.[\w-]+')
CURSOR = re.compile(r'"next":"[^"]+"')

def norm(text):
    text = TS.sub('<ts>', text)
    text = JWT.sub('<jwt>', text)
    return CURSOR.sub('"next":"<cursor>"', text)

class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *a, **k):
        return None

OPENER = urllib.request.build_opener(NoRedirect)

class Client:
    def __init__(self, base):
        self.base = base.rstrip('/')
        self.log = []
        self.token = None
        # Values each backend generates at random, replaced by a name.
        self.subst = {}

    def call(self, label, method, path, body=None, token=None, raw=False):
        req = urllib.request.Request(self.base + path, method=method)
        req.add_header('content-type', 'application/json')
        tok = token if token is not None else self.token
        if tok:
            req.add_header('authorization', 'Bearer ' + tok)
        data = json.dumps(body).encode() if body is not None else None
        try:
            with OPENER.open(req, data) as r:
                status, text, h = r.status, r.read().decode(), r.headers
        except urllib.error.HTTPError as e:
            status, text, h = e.code, e.read().decode(), e.headers
        ctype = (h.get('content-type') or '').split(';')[0]
        loc = h.get('location') or ''
        shown = norm(text + ('  -> ' + loc if loc else ''))
        for k, v in self.subst.items():
            shown = shown.replace(k, v)
        self.log.append((label, method, norm(self.sub(path)), status, ctype, shown))
        try:
            return status, json.loads(text) if text else None
        except ValueError:
            return status, text

    def sub(self, text):
        for k, v in self.subst.items():
            text = text.replace(k, v)
        return text

# ---------------------------------------------------------------- scenarios

def shortener(c, app_dir):
    s, link = c.call('create', 'POST', '/api/links', {'url': 'https://example.com/a'})
    code = link.get('code') if isinstance(link, dict) else None
    if code:
        c.subst[code] = '<code>' 
    c.call('create bad url', 'POST', '/api/links', {'url': 'javascript:alert(1)'})
    c.call('create no body', 'POST', '/api/links', {})
    if code:
        c.call('get link', 'GET', f'/api/links/{code}')
        c.call('follow', 'GET', f'/{code}')
        c.call('follow again', 'GET', f'/{code}')
        c.call('get link after hits', 'GET', f'/api/links/{code}')
    c.call('missing', 'GET', '/api/links/zzzzzz')
    c.call('follow missing', 'GET', '/zzzzzz')
    # `insert … buffered` flushes on its own clock; read after it has.
    time.sleep(2.5)
    c.call('stats', 'GET', '/api/v1/stats')
    c.call('og', 'GET', '/og.svg')

def mywallet(c, app_dir):
    reg = {'email': 'a@example.com', 'password': 'correct horse battery', 'first_name': 'Ann', 'last_name': 'Lee'}
    c.call('register', 'POST', '/auth/register', reg)
    c.call('register dup', 'POST', '/auth/register', reg)
    c.call('register bad email', 'POST', '/auth/register', dict(reg, email='nope'))
    s, t = c.call('login', 'POST', '/auth/login', {'email': 'a@example.com', 'password': 'correct horse battery'})
    c.call('login wrong', 'POST', '/auth/login', {'email': 'a@example.com', 'password': 'nope nope nope'})
    if isinstance(t, dict):
        c.token = t.get('token') or t.get('access_token')
    c.call('no wallets', 'GET', '/wallets')
    s, w = c.call('wallet', 'POST', '/wallets', {'name': 'Cash', 'currency': 'UZS', 'balance': 150000})
    c.call('wallet 2', 'POST', '/wallets', {'name': 'Card', 'currency': 'USD'})
    c.call('wallet bad', 'POST', '/wallets', {'name': '', 'currency': 'USD'})
    wid = w.get('id') if isinstance(w, dict) else 1
    c.call('wallets', 'GET', '/wallets')
    c.call('wallet one', 'GET', f'/wallets/{wid}')
    c.call('wallet patch', 'PATCH', f'/wallets/{wid}', {'name': 'Naqd'})
    s, cat = c.call('category', 'POST', '/categories', {'name': 'Food', 'description': 'eating'})
    cid = cat.get('id') if isinstance(cat, dict) else 1
    c.call('category 2', 'POST', '/categories', {'name': 'Salary'})
    c.call('category dup', 'POST', '/categories', {'name': 'Food'})
    c.call('categories', 'GET', '/categories')
    c.call('category one', 'GET', f'/categories/{cid}')
    c.call('category patch', 'PATCH', f'/categories/{cid}', {'description': 'groceries'})
    for i, amt in enumerate([12000, 7999, 100]):
        c.call(f'tx {i}', 'POST', f'/wallets/{wid}/transactions', {'amount': amt, 'category_id': cid, 'description': f'n{i}'})
    c.call('tx income', 'POST', f'/wallets/{wid}/transactions', {'amount': 50000, 'category_id': cid, 'is_income': True})
    c.call('tx zero', 'POST', f'/wallets/{wid}/transactions', {'amount': 0, 'category_id': cid})
    c.call('txs', 'GET', f'/wallets/{wid}/transactions')
    c.call('tx one', 'GET', '/transactions/1')
    c.call('wallet after', 'GET', f'/wallets/{wid}')
    c.call('categories after', 'GET', '/categories')
    c.call('stats', 'GET', '/stats')
    c.call('tx delete', 'DELETE', '/transactions/1')
    c.call('wallet after delete', 'GET', f'/wallets/{wid}')
    c.call('category delete', 'DELETE', f'/categories/{cid}')
    c.call('wallet delete', 'DELETE', f'/wallets/{wid}')
    c.call('missing wallet', 'GET', '/wallets/999')
    c.token = None
    c.call('no token', 'GET', '/wallets')

def eschool(c, app_dir):
    """e-school's own seed script, every call routed through the harness,
    then a read of the GETs the seed made reachable."""
    import os, random
    seed = open(os.path.join(app_dir, 'scripts', 'seed_demo.py')).read()
    src = seed.split("# ---- the credentials file")[0]
    # Replace the script's `call` with one that records through `c`.
    src = re.sub(r"def call\(method, path, body=None, token=None, ok=.*?\n(?=\n\ndef login)",
                 "def call(method, path, body=None, token=None, ok=None):\n"
                 "    return C.call(f'{method} {path}', method, '/api/v1' + path, body, token or '')\n",
                 src, flags=re.S)
    g = {'C': c, '__name__': 'seed', '__file__': '/nonexistent/seed.py'}
    random.seed(2026)
    exec(compile(src, 'seed_demo', 'exec'), g)
    admin = g['admin']
    # Every GET the admin can make, with the ids the seed created.
    for path in ['/me', '/admin/years', '/admin/users?role=teacher', '/admin/users?role=student',
                 '/admin/classes', '/admin/subjects', '/admin/schedule', '/announcements',
                 '/calendar/terms', '/admin/users/1', '/admin/classes/1', '/admin/classes/1/roster']:
        c.call('read ' + path, 'GET', '/api/v1' + path, token=admin)
    tk = g.get('tk')
    if tk:
        for path in ['/me', '/teacher/assignments', '/teacher/timetable']:
            c.call('teacher ' + path, 'GET', '/api/v1' + path, token=tk)


SCENARIOS = {'shortener': shortener, 'mywallet': mywallet, 'eschool': eschool}

if __name__ == '__main__':
    name, app_dir, a, b = sys.argv[1:5]
    scenario = SCENARIOS[name]
    ca, cb = Client(a), Client(b)
    scenario(ca, app_dir)
    scenario(cb, app_dir)
    # A value generated at random is only known after the call that made it.
    ca.log = [tuple(ca.sub(str(v)) if i == 5 else v for i, v in enumerate(e)) for e in ca.log]
    cb.log = [tuple(cb.sub(str(v)) if i == 5 else v for i, v in enumerate(e)) for e in cb.log]
    same = diff = 0
    for x, y in zip(ca.log, cb.log):
        if x == y:
            same += 1
        else:
            diff += 1
            print(f'--- {x[0]}: {x[1]} {x[2]}')
            print(f'  serve : {x[3]} {x[4]} {x[5][:400]}')
            print(f'  native: {y[3]} {y[4]} {y[5][:400]}')
    if len(ca.log) != len(cb.log):
        diff += 1
        print(f'!! different number of calls: {len(ca.log)} vs {len(cb.log)}')
    statuses = {}
    for x in ca.log:
        statuses[x[3]] = statuses.get(x[3], 0) + 1
    print(f'{name}: {same} identical, {diff} different, of {len(ca.log)} calls; statuses {statuses}')
    sys.exit(1 if diff else 0)
