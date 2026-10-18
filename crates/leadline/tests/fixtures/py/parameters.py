def parameters(a, b=1, *args, **kwargs):
    return a


def keyword_only(a, *, b, c):
    return a


def positional_only(a, /, b):
    return a
