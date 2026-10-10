import unittest

from calc import CalcError, run


class Examples(unittest.TestCase):
    def test_arithmetic(self):
        self.assertEqual(run("print 1 + 2 * 3, 7 / 2, 7 // 2"), ["7 3.5 3"])

    def test_function(self):
        src = """
fn square(x)
  return x * x
end
print square(12)
"""
        self.assertEqual(run(src), ["144"])

    def test_error_line(self):
        with self.assertRaises(CalcError) as cm:
            run('let a = 1\nprint a + "x"')
        self.assertEqual(str(cm.exception), "line 2: type error: cannot apply + to int and str")


if __name__ == "__main__":
    unittest.main()
