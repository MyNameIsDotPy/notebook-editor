"""Source-preserving Python operations. Never executes notebook code."""
import ast
import io
import json
import sys
import tokenize


def reindent(source, selected, spaces):
    # Lines beginning inside a string token must remain byte-for-byte intact.
    protected = set()
    for token in tokenize.generate_tokens(io.StringIO(source).readline):
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
        compile(source, '<original>', 'exec')
        nl = '\r\n' if '\r\n' in source else '\n'
        header = ''
        for key, label in [('description', 'Descripción'), ('input', 'Entrada'), ('output', 'Salida')]:
            if data.get(key) is not None:
                header += ''.join('# ' + label + ': ' + line + nl for line in data[key].split('\n'))
        body = reindent(source, set(range(len(source.splitlines()))), 4)
        info = '    logger.info(' + repr(data['logger_info']) + ')' + nl if data.get('logger_info') is not None else ''
        if not any(t.type not in (tokenize.COMMENT, tokenize.NL, tokenize.NEWLINE, tokenize.ENDMARKER, tokenize.INDENT, tokenize.DEDENT) for t in tokenize.generate_tokens(io.StringIO(source).readline)):
            body += '    pass' + nl
        if body and not body.endswith('\n'):
            body += nl
        result = header + 'try:' + nl + info + body + 'except Exception as error:' + nl
        result += '    logger.error(' + repr(data.get('logger_error') or 'Error en la celda') + ', exc_info=True)' + nl + '    raise' + nl
        compile(result, '<wrapped>', 'exec')
        return result
    issues = []
    for cell in data['cells']:
        source = cell['source']
        try:
            compile(source, '<cell>', 'exec')
        except SyntaxError as error:
            issues.append({'cell': cell['cell'], 'line': error.lineno, 'column': error.offset, 'message': error.msg})
            continue
        if mode != 'lint':
            continue
        tree = ast.parse(source)
        nodes = list(ast.walk(tree))
        comments = [t.string for t in tokenize.generate_tokens(io.StringIO(source).readline) if t.type == tokenize.COMMENT]
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
