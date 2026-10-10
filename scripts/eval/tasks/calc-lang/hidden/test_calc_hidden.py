import os
import subprocess
import sys
import tempfile
import unittest

from calc import CalcError, run


def err(src):
    try:
        run(src)
    except CalcError as e:
        return str(e)
    raise AssertionError("no CalcError for: " + src)


class Values(unittest.TestCase):
    def test_literals_and_printing(self):
        self.assertEqual(run('print 42, 1.5, "hi", true, false, nil'), ["42 1.5 hi true false nil"])

    def test_float_repr(self):
        self.assertEqual(run("print 4 / 2, 0.1 + 0.2, 10.0 ** 20"), ["2.0 0.30000000000000004 1e+20"])

    def test_big_ints(self):
        self.assertEqual(run("print 2 ** 100"), [str(2 ** 100)])

    def test_string_escapes(self):
        self.assertEqual(run(r'print "a\tb\"c\\d"'), ['a\tb"c\\d'])
        self.assertEqual(run(r'print "x\ny"'), ["x\ny"])

    def test_print_empty_and_multiple(self):
        self.assertEqual(run('print\nprint 1, 2;print "a" + "b"'), ["", "1 2", "ab"])

    def test_functions_print(self):
        self.assertEqual(run("fn f() return 1 end\nprint f, len"), ["<fn f> <fn len>"])

    def test_comments_and_separators(self):
        self.assertEqual(run("# start\nlet a = 1 # one\n;;\n\nprint a ; print a + 1 # done"), ["1", "2"])


class Operators(unittest.TestCase):
    def test_precedence(self):
        self.assertEqual(run("print 2 + 3 * 4 ** 2, (2 + 3) * 4, -2 ** 2, 2 ** -1, 2 ** 3 ** 2"), ["50 20 -4 0.5 512"])

    def test_floor_and_mod(self):
        self.assertEqual(run("print -7 // 2, -7 % 3, 7.5 // 2, 7 % -3"), ["-4 2 3.0 -2"])

    def test_string_ops(self):
        self.assertEqual(run('print "ab" * 3, 2 * "x", "a" < "b", "b" <= "a"'), ["ababab xx true false"])

    def test_equality(self):
        self.assertEqual(run('print 1 == 1.0, 1 == "1", nil == nil, true != false, "a" == "a", 0 == false'),
                         ["true false true true true false"])

    def test_logic_returns_operands(self):
        self.assertEqual(run('print nil or 3, 0 and 5, 1 and 2, "" or "x", not 0, not "a"'), ["3 0 2 x true false"])

    def test_short_circuit(self):
        self.assertEqual(run("print false and missing, true or missing"), ["false true"])

    def test_truthiness(self):
        src = 'if 0.0 then print 1 else print 2 end\nif "" then print 3 else print 4 end\nif "0" then print 5 end'
        self.assertEqual(run(src), ["2", "4", "5"])

    def test_not_binds_looser_than_comparison(self):
        self.assertEqual(run("print not 1 == 2"), ["true"])


class Control(unittest.TestCase):
    def test_if_elif_else(self):
        src = """
fn grade(n)
  if n >= 90 then return "A"
  elif n >= 80 then return "B"
  elif n >= 70 then return "C"
  else return "F"
  end
end
print grade(95), grade(85), grade(75), grade(10)
"""
        self.assertEqual(run(src), ["A B C F"])

    def test_while(self):
        src = "let i = 0\nlet s = 0\nwhile i < 10 do\n  i = i + 1\n  s = s + i\nend\nprint s"
        self.assertEqual(run(src), ["55"])

    def test_block_scope(self):
        src = "let x = 1\nif true then\n  let x = 2\n  print x\nend\nprint x"
        self.assertEqual(run(src), ["2", "1"])

    def test_assign_reaches_outer(self):
        src = "let x = 1\nif true then x = 5 end\nprint x"
        self.assertEqual(run(src), ["5"])

    def test_redeclare_same_scope(self):
        self.assertEqual(run("let a = 1\nlet a = a + 1\nprint a"), ["2"])

    def test_return_without_value(self):
        self.assertEqual(run("fn f()\n  return\nend\nprint f()"), ["nil"])

    def test_no_return_is_nil(self):
        self.assertEqual(run("fn f() let a = 1 end\nprint f()"), ["nil"])


class Functions(unittest.TestCase):
    def test_recursion(self):
        src = "fn fib(n)\n  if n < 2 then return n end\n  return fib(n - 1) + fib(n - 2)\nend\nprint fib(20)"
        self.assertEqual(run(src), ["6765"])

    def test_deep_recursion(self):
        src = "fn sum(n)\n  if n == 0 then return 0 end\n  return n + sum(n - 1)\nend\nprint sum(500)"
        self.assertEqual(run(src), ["125250"])

    def test_closures_counter(self):
        src = """
fn make_counter()
  let n = 0
  fn inc()
    n = n + 1
    return n
  end
  return inc
end
let a = make_counter()
let b = make_counter()
a()
a()
print a(), b()
"""
        self.assertEqual(run(src), ["3 1"])

    def test_higher_order(self):
        src = "fn twice(f, x) return f(f(x)) end\nfn add3(x) return x + 3 end\nprint twice(add3, 10)"
        self.assertEqual(run(src), ["16"])

    def test_closure_sees_later_assignment(self):
        src = "let x = 1\nfn get() return x end\nx = 7\nprint get()"
        self.assertEqual(run(src), ["7"])

    def test_call_chain(self):
        src = "fn adder(a)\n  fn add(b) return a + b end\n  return add\nend\nprint adder(2)(5)"
        self.assertEqual(run(src), ["7"])

    def test_builtins(self):
        src = 'print len("hello"), str(1.5) + "!", int(3.9), int(-3.9), int("-42"), abs(-5), min(3, 1, 2), max(1.5, 2)'
        self.assertEqual(run(src), ["5 1.5! 3 -3 -42 5 1 2"])

    def test_str_of_values(self):
        self.assertEqual(run('print str(nil) + str(true) + str(2.0)'), ["niltrue2.0"])


class Errors(unittest.TestCase):
    def test_type_error_binary(self):
        self.assertEqual(err('let a = 1\nprint a + "x"'), "line 2: type error: cannot apply + to int and str")
        self.assertEqual(err('print nil < 1'), "line 1: type error: cannot apply < to nil and int")
        self.assertEqual(err('print "a" - "b"'), "line 1: type error: cannot apply - to str and str")
        self.assertEqual(err('print 1.5 * "a"'), "line 1: type error: cannot apply * to float and str")

    def test_type_error_unary(self):
        self.assertEqual(err('print -"a"'), "line 1: type error: cannot apply - to str")

    def test_division_by_zero(self):
        self.assertEqual(err("print 1\nprint 1 / 0"), "line 2: division by zero")
        self.assertEqual(err("print 5 % 0"), "line 1: division by zero")
        self.assertEqual(err("print 5.0 // 0.0"), "line 1: division by zero")

    def test_undefined(self):
        self.assertEqual(err("print 1\n\nprint y"), "line 3: undefined variable 'y'")
        self.assertEqual(err("z = 3"), "line 1: undefined variable 'z'")

    def test_call_errors(self):
        self.assertEqual(err("let a = 3\nprint a(1)"), "line 2: not callable")
        self.assertEqual(err("fn f(x) return x end\n\nprint f(1, 2)"), "line 3: expected 1 arguments but got 2")
        self.assertEqual(err('print int("4x")'), "line 1: type error: cannot convert str to int")

    def test_return_outside(self):
        self.assertEqual(err("print 1\nreturn 2"), "line 2: return outside function")

    def assertEndOfInput(self, src):
        # SPEC.md's "(`end of input` instead of a quoted token)" reads either way: quoted or not
        self.assertIn(err(src), ("line 1: unexpected token 'end of input'", "line 1: unexpected token end of input"))

    def test_syntax_errors(self):
        self.assertEndOfInput("print 1 +")
        self.assertEndOfInput("print (1 + 2")
        self.assertEqual(err("let = 3"), "line 1: unexpected token '='")
        self.assertEqual(err("print 1 < 2 < 3"), "line 1: unexpected token '<'")
        self.assertEqual(err('print "abc'), "line 1: unterminated string")
        self.assertEqual(err("print 1\nprint 2 $ 3"), "line 2: unexpected character '$'")
        self.assertEndOfInput("if 1 then print 2")

    def test_error_stops_program(self):
        with self.assertRaises(CalcError):
            run("print 1\nprint 1 / 0\nprint 3")


class CommandLine(unittest.TestCase):
    def test_cli(self):
        with tempfile.TemporaryDirectory() as d:
            ok = os.path.join(d, "ok.calc")
            bad = os.path.join(d, "bad.calc")
            with open(ok, "w") as f:
                f.write('print "hello", 1 + 1\n')
            with open(bad, "w") as f:
                f.write("print 1\nprint nope\n")
            r = subprocess.run([sys.executable, "-m", "calc", ok], capture_output=True, text=True)
            self.assertEqual((r.returncode, r.stdout), (0, "hello 2\n"))
            r = subprocess.run([sys.executable, "-m", "calc", bad], capture_output=True, text=True)
            self.assertEqual(r.returncode, 1)
            self.assertIn("line 2: undefined variable 'nope'", r.stderr)


if __name__ == "__main__":
    unittest.main()
