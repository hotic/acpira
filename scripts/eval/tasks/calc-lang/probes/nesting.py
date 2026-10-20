# Post-hoc probe, not part of the scored hidden tests: deeply nested valid programs against a finished work tree.
# SPEC.md asks run() to return the printed lines or raise CalcError, so a RecursionError escaping run() is a defect;
# the reference solution passes every case. Usage: python3 -I probes/nesting.py WORK_TREE
import sys

sys.path.insert(0, sys.argv[1])
from calc import CalcError, run  # noqa: E402

CASES = {
    'parens80': ('print ' + '(' * 80 + '1' + ')' * 80, ['1']),
    'parens300': ('print ' + '(' * 300 + '1' + ')' * 300, ['1']),
    'not1000': ('print ' + 'not ' * 1000 + 'true', ['true']),
    'if500': ('if true then\n' * 500 + 'print 7\n' + 'end\n' * 500, ['7']),
    'calls100': ('fn id(x) return x end\nprint ' + 'id(' * 100 + '1' + ')' * 100, ['1']),
}

passed = 0
cells = []
for name, (source, want) in CASES.items():
    try:
        got = run(source)
        verdict = 'ok' if got == want else 'wrong'
    except CalcError:
        verdict = 'CalcError'
    except RecursionError:
        verdict = 'RecursionError'
    except Exception as e:  # any other crash counts against the solution too
        verdict = type(e).__name__
    passed += verdict == 'ok'
    cells.append(f'{name}={verdict}')
print(f'{passed}/{len(CASES)} ' + ' '.join(cells))
