"""Source-preserving Python operations. Never executes notebook code."""
import ast
import io
import json
import sys
import tokenize


def validation_source(source):
    """Mask explicit IPython commands, preserving Python line numbers.

    Token boundaries prevent strings and continued modulo expressions from
    being mistaken for magics. Magic arguments/bodies are opaque, not Python.
    """
    lines = source.splitlines(keepends=True)
    cell_magic = False
    while True:
        statement_start = True
        replacement = None
        try:
            for token in tokenize.generate_tokens(io.StringIO(''.join(lines)).readline):
                if token.type in (tokenize.ENCODING, tokenize.INDENT, tokenize.DEDENT,
                                  tokenize.COMMENT, tokenize.NL):
                    continue
                if token.type == tokenize.ERRORTOKEN and token.string.isspace():
                    continue
                if token.type == tokenize.NEWLINE:
                    statement_start = True
                    continue
                if statement_start and token.string in ('%', '!'):
                    row, col = token.start
                    text = lines[row - 1]
                    if text[:col].strip():
                        statement_start = False
                        continue
                    end = row
                    if text[col:].startswith('%%'):
                        # A cell magic consumes its entire remaining payload.
                        end = len(lines)
                        cell_magic = True
                    else:
                        while end < len(lines) and lines[end - 1].rstrip('\r\n').endswith('\\'):
                            end += 1
                    replacement = (row - 1, end, col)
                    break
                statement_start = False
        except (tokenize.TokenError, IndentationError, SyntaxError):
            # Leave real Python errors for compile(), with original positions.
            return ''.join(lines), cell_magic
        if replacement is None:
            return ''.join(lines), cell_magic
        start, end, col = replacement
        for i in range(start, end):
            ending = '\r\n' if lines[i].endswith('\r\n') else ('\n' if lines[i].endswith('\n') else '')
            lines[i] = (' ' * col + 'pass' if i == start else '') + ending


def reindent(source, selected, spaces):
    # Lines beginning inside a string token must remain byte-for-byte intact.
    protected = set()
    checked, cell_magic = validation_source(source)
    if cell_magic:
        raise ValueError('Cannot reindent or wrap a %% cell magic without changing its semantics')
    for token in tokenize.generate_tokens(io.StringIO(checked).readline):
        if token.type == tokenize.STRING or tokenize.tok_name.get(token.type) in ('FSTRING_MIDDLE', 'TSTRING_MIDDLE'):
            protected.update(range(token.start[0] + 1, token.end[0] + 1))
    return ''.join((' ' * spaces if i in selected and i + 1 not in protected and line.strip() else '') + line
                   for i, line in enumerate(source.splitlines(keepends=True)))


def operate(data):
    mode = data['mode']
    if mode == 'indent':
        return reindent(data['source'], set(data['selected']), data['spaces'])
    if mode == 'wrap':
        source = data['source']
        checked, cell_magic = validation_source(source)
        if cell_magic:
            raise ValueError('Cannot wrap a %% cell magic without changing its semantics')
        compile(checked, '<original>', 'exec')
        nl = '\r\n' if '\r\n' in source else '\n'
        header = ''
        for key, label in [('description', 'Descripción'), ('input', 'Entrada'), ('output', 'Salida')]:
            if data.get(key) is not None:
                header += ''.join('# ' + label + ': ' + line + nl for line in data[key].split('\n'))
        body = reindent(source, set(range(len(source.splitlines()))), 4)
        info = '    logger.info(' + repr(data['logger_info']) + ')' + nl if data.get('logger_info') is not None else ''
        if not any(t.type not in (tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE, tokenize.ENDMARKER, tokenize.INDENT, tokenize.DEDENT) for t in tokenize.generate_tokens(io.StringIO(checked).readline)):
            body += '    pass' + nl
        if body and not body.endswith('\n'):
            body += nl
        result = header + 'try:' + nl + info + body + 'except Exception as error:' + nl
        result += '    logger.error(' + repr(data.get('logger_error') or 'Error en la celda') + ', exc_info=True)' + nl + '    raise' + nl
        compile(validation_source(result)[0], '<wrapped>', 'exec')
        return result
    issues = []
    for cell in data['cells']:
        source = cell['source']
        checked, _ = validation_source(source)
        try:
            compile(checked, '<cell>', 'exec')
        except SyntaxError as error:
            issues.append({'cell': cell['cell'], 'line': error.lineno, 'column': error.offset, 'message': error.msg})
            continue
        if mode != 'lint':
            continue
        tree = ast.parse(checked)
        nodes = list(ast.walk(tree))
        comments = [t.string for t in tokenize.generate_tokens(io.StringIO(checked).readline) if t.type == tokenize.COMMENT]
        found = {name: any(label in comment for comment in comments) for name, label in [('description', 'Descripción'), ('input', 'Entrada'), ('output', 'Salida')]}
        found.update({'try': any(isinstance(n, ast.Try) for n in nodes), 'except': any(isinstance(n, ast.ExceptHandler) for n in nodes), 'raise': any(isinstance(n, ast.Raise) for n in nodes)})
        for name in ('info', 'error'):
            found['logger.' + name] = any(isinstance(n, ast.Call) and isinstance(n.func, ast.Attribute) and isinstance(n.func.value, ast.Name) and n.func.value.id == 'logger' and n.func.attr == name for n in nodes)
        for rule in data['required']:
            if not found[rule]:
                issues.append({'cell': cell['cell'], 'message': 'Missing ' + rule})
    return issues


try:
    print(json.dumps({'result': operate(json.load(sys.stdin))}))
except Exception as error:
    print(json.dumps({'error': str(error)}))
    sys.exit(1)
