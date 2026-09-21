def route(command):
    match command.split():
        case ["go", direction]:
            return direction
        case ["look"]:
            return "look"
        case _:
            return "unknown"
