# calc: a small scripting language

`calc.run(source)` executes a program and returns the lines it printed, as a list of strings. Any error stops the
program and raises `calc.CalcError` whose message is `line N: <message>`. Lines are numbered from 1.

## Lexical structure

- Statements are separated by newlines or `;`. Blank lines and empty statements are allowed.
- `#` starts a comment that runs to the end of the line (outside strings).
- Integers: `0`, `42` (arbitrary size). Floats: `1.5`, `0.25` (digits on both sides of the dot).
- Strings: double quotes, with the escapes `\n`, `\t`, `\"` and `\\`. A string cannot span lines.
- Names: letters, digits and `_`, not starting with a digit.
- Keywords: `let print if then else elif end while do fn return and or not true false nil`.

## Values

int, float, string, bool (`true` / `false`), `nil`, and functions.

Printed form: ints in decimal; floats as Python's `repr` (`2.0`, `0.1`, `1e+20`); strings without quotes; `true`,
`false`, `nil`; functions as `<fn NAME>`.

Truthiness: `false`, `nil`, `0`, `0.0` and `""` are false; everything else is true.

## Statements

- `let NAME = EXPR` declares a variable in the current scope (declaring the same name again in the same scope is allowed
  and replaces it).
- `NAME = EXPR` assigns to the nearest enclosing variable of that name; error `undefined variable 'NAME'` if none.
- `print` followed by zero or more expressions separated by commas: prints them joined by one space (`print` alone
  prints an empty line).
- `if EXPR then BLOCK (elif EXPR then BLOCK)* (else BLOCK)? end`
- `while EXPR do BLOCK end`
- `fn NAME(PARAMS) BLOCK end` declares a function in the current scope, like `let`.
- `return EXPR?` returns from the innermost function (`nil` without an expression). Outside a function: error
  `return outside function`.
- Any expression on its own is a statement (its value is discarded).

Every block (`if`/`elif`/`else` branches, `while` bodies, function bodies) opens a new scope. Functions are closures:
they see the variables of the scope they were declared in, by reference.

## Expressions

Precedence, lowest first:

1. `or`
2. `and`
3. `not` (prefix)
4. `==` `!=` `<` `<=` `>` `>=` (do not chain: `a < b < c` is a parse error at the second operator)
5. `+` `-`
6. `*` `/` `//` `%`
7. unary `-`
8. `**` (right associative; `-2 ** 2` is `-4`)
9. calls `f(a, b)`, parentheses, literals, names

- `and` / `or` short-circuit and return the deciding operand (`nil or 3` is `3`, `0 and x` is `0`).
- `not` returns a bool.
- Arithmetic on ints and floats mixes like Python. `/` always gives a float; `//` and `%` floor like Python. `**` with
  ints and a non-negative exponent gives an int.
- Division or modulo by zero (`/`, `//`, `%`): error `division by zero`.
- `+` also concatenates two strings; `*` repeats a string by an int (either order).
- `==` / `!=` compare any two values (different types are unequal, except int and float that compare by value).
- `<` `<=` `>` `>=` compare two numbers or two strings.
- Any other operand types: error `type error: cannot apply OP to T1 and T2` (unary: `cannot apply OP to T`), where the
  type names are `int float str bool nil fn`. Example: `type error: cannot apply + to str and int`.

## Functions and built-ins

- Calling a non-function: error `not callable`. Wrong argument count: `expected N arguments but got M`.
- Recursion at least 500 calls deep must work.
- Built-ins: `len(s)` (length of a string), `str(x)` (printed form), `int(x)` (from a float, truncating toward zero, or
  from a string of digits with an optional leading `-`; anything else: `type error: cannot convert T to int`),
  `abs(x)`, `min(a, b, ...)` and `max(a, b, ...)` (one or more numbers).

## Errors

- Syntax errors: `unexpected token 'TEXT'` for the first token that does not fit (`end of input` instead of a quoted
  token at the end), `unterminated string`, `unexpected character 'C'`.
- Undefined name in an expression: `undefined variable 'NAME'`.
- The line in the message is the line of the token where the error was detected: the operator for type errors and
  division by zero, the name for undefined variables, the opening parenthesis of a call for call errors.

## Command line

`python3 -m calc FILE` runs a program and prints its output; on error it prints the message to stderr and exits with
status 1.
