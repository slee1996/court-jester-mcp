def _cj_native_value(value):
    def expression(item, depth=0):
        if depth > 16:
            raise ValueError('native snapshot exceeds supported depth')
        if type(item) is float:
            if item != item:
                return "float('nan')"
            if item == float('inf'):
                return "float('inf')"
            if item == float('-inf'):
                return "float('-inf')"
        if type(item) is list:
            return '[' + ', '.join(expression(child, depth + 1) for child in item) + ']'
        if type(item) not in (type(None), bool, int, float, str, bytes, bytearray):
            raise ValueError('unsupported native snapshot value')
        return repr(item)
    snapshot = {"expression": expression(value)}
    try:
        encoded = _cj_native_json.dumps(value, ensure_ascii=False, allow_nan=False)
        encoded.encode('utf-8')
        snapshot["json_value"] = _cj_native_json.loads(encoded)
    except Exception:
        pass
    return snapshot
