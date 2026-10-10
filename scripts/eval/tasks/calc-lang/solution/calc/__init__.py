"""Reference solution for the calc-lang eval task (never copied into the workspace)."""
import sys

__all__ = ["CalcError", "run"]

sys.setrecursionlimit(20000)


class CalcError(Exception):
    pass


KEYWORDS = {"let", "print", "if", "then", "else", "elif", "end", "while", "do", "fn", "return", "and", "or", "not",
            "true", "false", "nil"}
OPS = ["**", "//", "==", "!=", "<=", ">=", "+", "-", "*", "/", "%", "<", ">", "(", ")", ",", "=", ";"]


class Tok:
    def __init__(self, kind, text, value, line):
        self.kind, self.text, self.value, self.line = kind, text, value, line


def err(line, msg):
    return CalcError(f"line {line}: {msg}")


def lex(src):
    toks, i, line, n = [], 0, 1, len(src)
    while i < n:
        c = src[i]
        if c == "\n":
            toks.append(Tok("nl", "\n", None, line))
            line += 1
            i += 1
        elif c in " \t\r":
            i += 1
        elif c == "#":
            while i < n and src[i] != "\n":
                i += 1
        elif c.isdigit():
            j = i
            while j < n and src[j].isdigit():
                j += 1
            if j + 1 < n and src[j] == "." and src[j + 1].isdigit():
                j += 1
                while j < n and src[j].isdigit():
                    j += 1
                toks.append(Tok("num", src[i:j], float(src[i:j]), line))
            else:
                toks.append(Tok("num", src[i:j], int(src[i:j]), line))
            i = j
        elif c.isalpha() or c == "_":
            j = i
            while j < n and (src[j].isalnum() or src[j] == "_"):
                j += 1
            word = src[i:j]
            toks.append(Tok("kw" if word in KEYWORDS else "name", word, word, line))
            i = j
        elif c == '"':
            j, out = i + 1, []
            while True:
                if j >= n or src[j] == "\n":
                    raise err(line, "unterminated string")
                ch = src[j]
                if ch == '"':
                    break
                if ch == "\\" and j + 1 < n:
                    esc = {"n": "\n", "t": "\t", '"': '"', "\\": "\\"}.get(src[j + 1])
                    if esc is not None:
                        out.append(esc)
                        j += 2
                        continue
                out.append(ch)
                j += 1
            toks.append(Tok("str", src[i:j + 1], "".join(out), line))
            i = j + 1
        else:
            for op in OPS:
                if src.startswith(op, i):
                    toks.append(Tok("op", op, op, line))
                    i += len(op)
                    break
            else:
                raise err(line, f"unexpected character '{c}'")
    toks.append(Tok("eof", "", None, line))
    return toks


class Parser:
    def __init__(self, toks):
        self.toks, self.i = toks, 0

    def peek(self):
        return self.toks[self.i]

    def next(self):
        t = self.toks[self.i]
        self.i += 1
        return t

    def at(self, text):
        t = self.peek()
        return t.kind in ("op", "kw") and t.text == text

    def unexpected(self, t=None):
        t = t or self.peek()
        return err(t.line, "unexpected token 'end of input'" if t.kind == "eof" else
                   "unexpected token '\\n'" if t.kind == "nl" else f"unexpected token '{t.text}'")

    def expect(self, text):
        if not self.at(text):
            raise self.unexpected()
        return self.next()

    def skip_seps(self):
        while self.peek().kind == "nl" or self.at(";"):
            self.next()

    def program(self):
        body = self.block(())
        if self.peek().kind != "eof":
            raise self.unexpected()
        return body

    def block(self, enders):
        stmts = []
        self.skip_seps()
        while self.peek().kind != "eof" and not any(self.at(e) for e in enders):
            stmts.append(self.statement())
            t = self.peek()
            if t.kind == "nl" or self.at(";"):
                self.skip_seps()
            elif t.kind == "eof" or any(self.at(e) for e in enders):
                pass
            else:
                raise self.unexpected()
        return stmts

    def statement(self):
        t = self.peek()
        if self.at("let"):
            self.next()
            name = self.next()
            if name.kind != "name":
                raise self.unexpected(name)
            self.expect("=")
            return ("let", name.text, self.expr())
        if self.at("print"):
            self.next()
            args = []
            if not (self.peek().kind in ("nl", "eof") or self.at(";") or self.at("end") or self.at("else") or self.at("elif")):
                args.append(self.expr())
                while self.at(","):
                    self.next()
                    args.append(self.expr())
            return ("print", args)
        if self.at("if"):
            self.next()
            arms = []
            cond = self.expr()
            self.expect("then")
            arms.append((cond, self.block(("elif", "else", "end"))))
            other = None
            while self.at("elif"):
                self.next()
                cond = self.expr()
                self.expect("then")
                arms.append((cond, self.block(("elif", "else", "end"))))
            if self.at("else"):
                self.next()
                other = self.block(("end",))
            self.expect("end")
            return ("if", arms, other)
        if self.at("while"):
            self.next()
            cond = self.expr()
            self.expect("do")
            body = self.block(("end",))
            self.expect("end")
            return ("while", cond, body)
        if self.at("fn"):
            self.next()
            name = self.next()
            if name.kind != "name":
                raise self.unexpected(name)
            self.expect("(")
            params = []
            if not self.at(")"):
                while True:
                    p = self.next()
                    if p.kind != "name":
                        raise self.unexpected(p)
                    params.append(p.text)
                    if self.at(","):
                        self.next()
                        continue
                    break
            self.expect(")")
            body = self.block(("end",))
            self.expect("end")
            return ("fn", name.text, params, body)
        if self.at("return"):
            self.next()
            value = None
            if not (self.peek().kind in ("nl", "eof") or self.at(";") or self.at("end") or self.at("else") or self.at("elif")):
                value = self.expr()
            return ("return", value, t.line)
        if t.kind == "name" and self.toks[self.i + 1].kind == "op" and self.toks[self.i + 1].text == "=":
            self.next()
            self.next()
            return ("assign", t.text, self.expr(), t.line)
        return ("expr", self.expr())

    def expr(self):
        return self.or_()

    def or_(self):
        left = self.and_()
        while self.at("or"):
            self.next()
            left = ("or", left, self.and_())
        return left

    def and_(self):
        left = self.not_()
        while self.at("and"):
            self.next()
            left = ("and", left, self.not_())
        return left

    def not_(self):
        if self.at("not"):
            self.next()
            return ("not", self.not_())
        return self.cmp()

    def cmp(self):
        left = self.add()
        if any(self.at(o) for o in ("==", "!=", "<", "<=", ">", ">=")):
            op = self.next()
            left = ("bin", op.text, left, self.add(), op.line)
            if any(self.at(o) for o in ("==", "!=", "<", "<=", ">", ">=")):
                raise self.unexpected()
        return left

    def add(self):
        left = self.mul()
        while self.at("+") or self.at("-"):
            op = self.next()
            left = ("bin", op.text, left, self.mul(), op.line)
        return left

    def mul(self):
        left = self.unary()
        while any(self.at(o) for o in ("*", "/", "//", "%")):
            op = self.next()
            left = ("bin", op.text, left, self.unary(), op.line)
        return left

    def unary(self):
        if self.at("-"):
            op = self.next()
            return ("neg", self.unary(), op.line)
        return self.power()

    def power(self):
        base = self.call()
        if self.at("**"):
            op = self.next()
            return ("bin", "**", base, self.unary(), op.line)
        return base

    def call(self):
        node = self.primary()
        while self.at("("):
            paren = self.next()
            args = []
            if not self.at(")"):
                args.append(self.expr())
                while self.at(","):
                    self.next()
                    args.append(self.expr())
            self.expect(")")
            node = ("call", node, args, paren.line)
        return node

    def primary(self):
        t = self.next()
        if t.kind == "num" or t.kind == "str":
            return ("lit", t.value)
        if t.kind == "kw" and t.text in ("true", "false", "nil"):
            return ("lit", {"true": True, "false": False, "nil": None}[t.text])
        if t.kind == "name":
            return ("name", t.text, t.line)
        if t.kind == "op" and t.text == "(":
            e = self.expr()
            self.expect(")")
            return e
        raise self.unexpected(t)


class Fn:
    def __init__(self, name, params, body, env):
        self.name, self.params, self.body, self.env = name, params, body, env


class Builtin:
    def __init__(self, name, f, arity):
        self.name, self.f, self.arity = name, f, arity


class Return(Exception):
    def __init__(self, value):
        self.value = value


def tname(v):
    if isinstance(v, bool):
        return "bool"
    if v is None:
        return "nil"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    return "fn"


def show(v):
    if v is True:
        return "true"
    if v is False:
        return "false"
    if v is None:
        return "nil"
    if isinstance(v, float):
        return repr(v)
    if isinstance(v, (Fn, Builtin)):
        return f"<fn {v.name}>"
    return str(v)


def truthy(v):
    return not (v is False or v is None or (tname(v) in ("int", "float") and v == 0) or v == "")


def isnum(v):
    return tname(v) in ("int", "float")


class Env:
    def __init__(self, parent=None):
        self.vars, self.parent = {}, parent

    def find(self, name):
        e = self
        while e is not None:
            if name in e.vars:
                return e
            e = e.parent
        return None


def to_int(line, x):
    if tname(x) == "int":
        return x
    if tname(x) == "float":
        return int(x)
    if tname(x) == "str":
        s = x[1:] if x.startswith("-") else x
        if s.isdigit() and s.isascii():
            return int(x)
    raise err(line, f"type error: cannot convert {tname(x)} to int")


def _minmax(pick):
    def f(line, args):
        if not args:
            raise err(line, "expected 1 arguments but got 0")
        for a in args:
            if not isnum(a):
                raise err(line, f"type error: cannot apply {pick.__name__} to {tname(a)}")
        return pick(args)
    return f


def _len(line, args):
    if tname(args[0]) != "str":
        raise err(line, f"type error: cannot apply len to {tname(args[0])}")
    return len(args[0])


def _abs(line, args):
    if not isnum(args[0]):
        raise err(line, f"type error: cannot apply abs to {tname(args[0])}")
    return abs(args[0])


BUILTINS = {
    "len": Builtin("len", _len, 1),
    "str": Builtin("str", lambda line, a: show(a[0]), 1),
    "int": Builtin("int", lambda line, a: to_int(line, a[0]), 1),
    "abs": Builtin("abs", _abs, 1),
    "min": Builtin("min", _minmax(min), None),
    "max": Builtin("max", _minmax(max), None),
}


class Interp:
    def __init__(self):
        self.out = []
        self.depth = 0

    def exec_block(self, stmts, env):
        for s in stmts:
            self.exec(s, env)

    def exec(self, s, env):
        k = s[0]
        if k == "let":
            env.vars[s[1]] = self.eval(s[2], env)
        elif k == "assign":
            value = self.eval(s[2], env)
            target = env.find(s[1])
            if target is None:
                raise err(s[3], f"undefined variable '{s[1]}'")
            target.vars[s[1]] = value
        elif k == "print":
            self.out.append(" ".join(show(self.eval(a, env)) for a in s[1]))
        elif k == "if":
            for cond, body in s[1]:
                if truthy(self.eval(cond, env)):
                    self.exec_block(body, Env(env))
                    return
            if s[2] is not None:
                self.exec_block(s[2], Env(env))
        elif k == "while":
            while truthy(self.eval(s[1], env)):
                self.exec_block(s[2], Env(env))
        elif k == "fn":
            env.vars[s[1]] = Fn(s[1], s[2], s[3], env)
        elif k == "return":
            if self.depth == 0:
                raise err(s[2], "return outside function")
            raise Return(None if s[1] is None else self.eval(s[1], env))
        else:
            self.eval(s[1], env)

    def eval(self, e, env):
        k = e[0]
        if k == "lit":
            return e[1]
        if k == "name":
            target = env.find(e[1])
            if target is not None:
                return target.vars[e[1]]
            if e[1] in BUILTINS:
                return BUILTINS[e[1]]
            raise err(e[2], f"undefined variable '{e[1]}'")
        if k == "or":
            left = self.eval(e[1], env)
            return left if truthy(left) else self.eval(e[2], env)
        if k == "and":
            left = self.eval(e[1], env)
            return self.eval(e[2], env) if truthy(left) else left
        if k == "not":
            return not truthy(self.eval(e[1], env))
        if k == "neg":
            v = self.eval(e[1], env)
            if not isnum(v):
                raise err(e[2], f"type error: cannot apply - to {tname(v)}")
            return -v
        if k == "bin":
            return self.binop(e[1], self.eval(e[2], env), self.eval(e[3], env), e[4])
        if k == "call":
            f = self.eval(e[1], env)
            args = [self.eval(a, env) for a in e[2]]
            return self.call(f, args, e[3])
        raise AssertionError(k)

    def call(self, f, args, line):
        if isinstance(f, Builtin):
            if f.arity is not None and len(args) != f.arity:
                raise err(line, f"expected {f.arity} arguments but got {len(args)}")
            return f.f(line, args)
        if not isinstance(f, Fn):
            raise err(line, "not callable")
        if len(args) != len(f.params):
            raise err(line, f"expected {len(f.params)} arguments but got {len(args)}")
        scope = Env(f.env)
        scope.vars.update(zip(f.params, args))
        self.depth += 1
        try:
            self.exec_block(f.body, scope)
        except Return as r:
            return r.value
        finally:
            self.depth -= 1
        return None

    def binop(self, op, a, b, line):
        ta, tb = tname(a), tname(b)
        bad = err(line, f"type error: cannot apply {op} to {ta} and {tb}")
        if op == "==":
            return (isnum(a) and isnum(b) and a == b) or (ta == tb and not isnum(a) and a == b)
        if op == "!=":
            return not self.binop("==", a, b, line)
        if op in ("<", "<=", ">", ">="):
            if not ((isnum(a) and isnum(b)) or (ta == tb == "str")):
                raise bad
            return {"<": a < b, "<=": a <= b, ">": a > b, ">=": a >= b}[op]
        if op == "+":
            if isnum(a) and isnum(b):
                return a + b
            if ta == tb == "str":
                return a + b
            raise bad
        if op == "*":
            if isnum(a) and isnum(b):
                return a * b
            if ta == "str" and tb == "int":
                return a * b
            if ta == "int" and tb == "str":
                return a * b
            raise bad
        if not (isnum(a) and isnum(b)):
            raise bad
        if op == "-":
            return a - b
        if op in ("/", "//", "%") and b == 0:
            raise err(line, "division by zero")
        if op == "/":
            return a / b
        if op == "//":
            return a // b
        if op == "%":
            return a % b
        if op == "**":
            return a ** b
        raise AssertionError(op)


def run(source):
    interp = Interp()
    try:
        interp.exec_block(Parser(lex(source)).program(), Env())
    except RecursionError:
        raise CalcError("line 0: recursion too deep")
    return interp.out
