#!/usr/bin/env python3
import json
import re
from pathlib import Path


def name(value):
    return value.casefold()


SWIFT_TYPES = ("swift.struct", "swift.class", "swift.enum", "swift.protocol", "swift.typealias")
UNIFFI_RUNTIME = re.compile(r"^(FfiConverter|Uniffi|RustBuffer|RustCallStatus|ForeignBytes|ForeignCallback|Disposable$|NoHandle$|ComposableSingletons)")
UNIFFI_HANDLE_TYPES = ("Lcom/noma4i/medialayer/NoHandle;", "Lcom/noma4i/medialayer/UniffiWithHandle;")
THROWABLE_PARENTS = ("java/lang/Exception", "java/lang/RuntimeException", "java/lang/Throwable")
COMPOSER = "Landroidx/compose/runtime/Composer;"
UNIFFI_FACADE = "MedialayerKt"
MARKER = "Lkotlin/jvm/internal/DefaultConstructorMarker;"
SWIFTUI_REQUIREMENTS = {"body", "makeUIView", "updateUIView", "dismantleUIView", "makeCoordinator"}
COMPILER_SHAPES = {"copy", "equals", "hashCode", "toString", "values", "valueOf", "getEntries"}


def case_labels(labels):
    return len(labels) or ()


def swift_symbols(graphs):
    entries = []
    seen = set()
    shapes = set()
    for graph in graphs:
        for symbol in json.loads(graph)["symbols"]:
            precise = symbol["identifier"]["precise"]
            if "::SYNTHESIZED::" in precise or precise in seen:
                continue
            seen.add(precise)
            shape = (tuple(symbol["pathComponents"]), symbol["kind"]["identifier"])
            if shape in shapes:
                continue
            shapes.add(shape)
            entries.append(symbol)
    types = {symbol["pathComponents"][-1] for symbol in entries if symbol["kind"]["identifier"] in SWIFT_TYPES}
    views = {symbol["pathComponents"][0] for symbol in entries if symbol["pathComponents"][-1] == "body" and len(symbol["pathComponents"]) == 2 and any("View" in fragment.get("spelling", "") for fragment in symbol.get("declarationFragments", []))}
    views |= {symbol["pathComponents"][0] for symbol in entries if symbol["pathComponents"][-1].startswith("makeUIView")}
    view_inits = set()
    properties = {(symbol["pathComponents"][:-1] and tuple(symbol["pathComponents"][:-1]), symbol["pathComponents"][-1]) for symbol in entries if symbol["kind"]["identifier"] in ("swift.property", "swift.type.property")}
    result = []
    for symbol in entries:
        path = symbol["pathComponents"]
        kind = symbol["kind"]["identifier"]
        owners = [item[: -len("Protocol")] for item in path if item.endswith("Protocol") and item[: -len("Protocol")] in types]
        if owners or any(item.endswith("Protocol") and item[: -len("Protocol")] in types for item in path):
            continue
        if path[0].startswith("uniffi") or (path[0].endswith("Impl") and path[0][: -len("Impl")] in types):
            continue
        owner = name(path[-2]) if len(path) > 1 else ""
        member = path[-1].split("(", 1)[0]
        if member in SWIFTUI_REQUIREMENTS and (member != "body" or path[0] in views):
            continue
        if kind == "swift.init" and path[0] in views and len(path) == 2:
            if path[0] not in view_inits:
                view_inits.add(path[0])
                result.append(((name(path[0]), "init", -2), "swift:" + path[0] + ".init"))
            continue
        setter = re.fullmatch(r"set([A-Z][A-Za-z0-9]*)\([^:)]*:\)", path[-1])
        if setter and kind in ("swift.method", "swift.type.method"):
            property_name = setter.group(1)[0].lower() + setter.group(1)[1:]
            if (tuple(path[:-1]), property_name) in properties:
                continue
            member = property_name
            arity = 0
        elif kind in SWIFT_TYPES:
            arity = -1
        elif kind == "swift.enum.case":
            arity = case_labels(re.findall(r"([A-Za-z0-9_]+):", path[-1]))
        else:
            arity = len(symbol.get("functionSignature", {}).get("parameters", []))
            if "functionSignature" not in symbol and "(" in path[-1]:
                arity = path[-1].count(":")
        if member.endswith("Stream") and arity != -1:
            member = member[: -len("Stream")]
        result.append(((owner, name(member), arity), "swift:" + ".".join(path) + " " + symbol["identifier"]["precise"]))
    return result


def descriptor_types(descriptor):
    return re.findall(r"\[*L[^;]+;|\[*[ZBCSIJFD]", descriptor)


def kotlin_classes(dumps):
    classes = []
    for dump in dumps:
        current = None
        for line in dump.splitlines():
            if not line.startswith("\t"):
                match = re.search(r"\bclass\s+([^\s:{]+)(?:\s*:\s*([^{]+))?", line)
                if match:
                    supers = [item.strip() for item in (match.group(2) or "").split(",") if item.strip()]
                    current = {"full": match.group(1), "supers": supers, "members": []}
                    classes.append(current)
                elif line.strip() and line.strip() != "}":
                    raise ValueError("unknown Kotlin API declaration: " + line)
                continue
            match = re.search(r"\b(fun|field)\s+([^\s]+)\s+([^\s]+)", line)
            if not match:
                raise ValueError("unknown Kotlin API member: " + line)
            current["members"].append(match.groups() + (line[: match.start()],))
    return classes


def kotlin_getter(member):
    base = member.split("-", 1)[0]
    if base.startswith(("get", "set")) and len(base) > 3 and base[3].isupper():
        return base[3:]
    return None


def kotlin_symbols(dumps):
    classes = kotlin_classes(dumps)
    by_name = {item["full"]: item for item in classes}
    interfaces = {item["full"] + "Interface" for item in classes}
    result = []
    composable_inits = set()
    for item in classes:
        full_owner = item["full"]
        owner = full_owner.split("/")[-1]
        parts = owner.split("$")
        generated = UNIFFI_RUNTIME.match(parts[0]) or parts[-1] == "ErrorHandler" or (full_owner in interfaces and any(full_owner in other["supers"] for other in classes))
        handle = "com/noma4i/medialayer/Disposable" in item["supers"]
        if handle and parts[-1].endswith("Impl") and full_owner[: -len("Impl")] in by_name:
            generated = True
        if generated:
            continue
        facade = owner.endswith("Kt")
        members = item["members"]
        member_names = {kind_member[1].split("-", 1)[0] for kind_member in members if kind_member[0] == "fun"}
        data_class = "copy" in member_names and any(re.fullmatch(r"component[1-9][0-9]*", entry) for entry in member_names)
        parent = item["supers"][0] if item["supers"] else ""
        sealed_case = parent in by_name and full_owner.startswith(parent + "$") and "$" not in full_owner[len(parent) + 1:]
        enum = "java/lang/Enum" in item["supers"]
        if sealed_case:
            labels = [kotlin_getter(entry[1]) for entry in members if entry[0] == "fun" and entry[2].startswith("()") and kotlin_getter(entry[1])]
            if any(entry in by_name[parent]["supers"] for entry in THROWABLE_PARENTS):
                labels = [label for label in labels if label != "Message"]
            result.append(((name(parts[-2]), name(parts[-1]), case_labels(labels)), "kotlin:" + full_owner))
            continue
        if parts[-1] == "DefaultImpls" and parts[-1] != owner:
            continue
        if not facade and parts[-1] != "Companion":
            container = name(parts[-2]) if len(parts) > 1 else ""
            result.append(((container, name(parts[-1]), -1), "kotlin:" + full_owner))
        current_owner = "" if facade else name(owner.replace("$Companion", "").split("$")[-1])
        for kind, member, descriptor, modifiers in members:
            base_member = member.split("-", 1)[0]
            if kind == "field" and (base_member in {"Companion", "INSTANCE", "$stable"}):
                continue
            if enum and kind == "field" and descriptor == "L" + full_owner + ";":
                result.append(((current_owner, name(base_member.replace("_", "")), ()), "kotlin:" + full_owner + "." + base_member))
                continue
            if (kotlin_getter(member) or base_member).casefold().startswith("uniffi") or descriptor.endswith("$ErrorHandler;"):
                continue
            if handle and (base_member in {"close", "destroy", "getCleanable", "getHandle"} or any(entry in descriptor for entry in UNIFFI_HANDLE_TYPES)):
                continue
            if "$default" in base_member:
                continue
            if data_class and kind == "fun" and (re.fullmatch(r"component[1-9][0-9]*", base_member) or base_member in COMPILER_SHAPES):
                continue
            if enum and kind == "fun" and base_member in COMPILER_SHAPES:
                continue
            if kind == "fun" and base_member in {"equals", "hashCode", "toString"}:
                continue
            args = descriptor_types(descriptor.split(")", 1)[0][1:]) if kind == "fun" else []
            if args and args[-1] == MARKER:
                args.pop()
                bridge = base_member == "<init>" and args and args[-1] == "I" and any(other[1] == "<init>" and MARKER in other[2] and len(descriptor_types(other[2].split(")", 1)[0][1:])) == len(args) for other in members)
                if bridge or base_member != "<init>" or any(other[1] == "<init>" and "synthetic" not in other[3] and len(descriptor_types(other[2].split(")", 1)[0][1:])) == len(args) - 1 for other in members):
                    continue
            if args and args[-1] == "Lkotlin/coroutines/Continuation;":
                args.pop()
            if "Landroidx/compose/runtime/Composer;" in args:
                args = args[:args.index("Landroidx/compose/runtime/Composer;")]
            member = base_member
            if kind == "field" and re.fullmatch(r"[A-Z][A-Z0-9_]+", member):
                result.append(((current_owner, name(member.replace("_", "")), 0), "kotlin:" + full_owner + "." + member + " " + descriptor))
                continue
            if facade and kind == "fun" and "static" in modifiers:
                if COMPOSER in descriptor and member[:1].isupper():
                    result.append((("", name(member), -1), "kotlin:" + full_owner + "." + member + " composable"))
                    if name(member) not in composable_inits:
                        composable_inits.add(name(member))
                        result.append(((name(member), "init", -2), "kotlin:" + full_owner + "." + member + " " + descriptor))
                    continue
                if args and args[0].startswith("Lcom/noma4i/medialayer/") and owner != UNIFFI_FACADE:
                    receiver = name(args[0][:-1].split("/")[-1].split("$")[0])
                    args = args[1:]
                    if kotlin_getter(member) and not args:
                        result.append(((receiver, name(kotlin_getter(member)), 0), "kotlin:" + full_owner + "." + member + " " + descriptor))
                    else:
                        stripped = member[: -len("Flow")] if member.endswith("Flow") and len(member) > 4 else member
                        result.append(((receiver, name(stripped), len(args)), "kotlin:" + full_owner + "." + member + " " + descriptor))
                    continue
            if member.startswith("set") and kind == "fun" and ("is" + member[3:]) in member_names:
                continue
            if member == "<init>":
                member = "init"
            elif kind == "fun" and kotlin_getter(member) and (descriptor.startswith("()") or (member.startswith("set") and len(args) == 1)):
                result.append(((current_owner, name(kotlin_getter(member)), 0), "kotlin:" + full_owner + "." + kotlin_getter(member)))
                continue
            result.append(((current_owner, name(member), len(args)), "kotlin:" + full_owner + "." + base_member + " " + descriptor))
    return list(dict.fromkeys(result))


def compare(swift_graphs, kotlin_dumps, exceptions_path):
    swift = swift_symbols(swift_graphs)
    kotlin = kotlin_symbols(kotlin_dumps)
    if not swift or not kotlin:
        raise ValueError("API schema contains no public symbols")
    exceptions = {}
    aliases = {}
    for line in Path(exceptions_path).read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        symbol, twin, reason = line.split("|", 2)
        if not symbol.strip() or not twin.strip() or not reason.strip():
            raise ValueError("API exception requires symbol, twin and reason")
        symbol, twin = symbol.strip(), twin.strip()
        kotlin_leaf = re.split(r"[/$]", symbol)[-1]
        if symbol.startswith("kotlin:") and twin.startswith("swift:") and not reason.strip().startswith("whole:") and "." not in kotlin_leaf and "." not in twin and " " not in twin:
            if name(kotlin_leaf) in aliases:
                raise ValueError("duplicate API twin: " + symbol)
            aliases[name(kotlin_leaf)] = name(twin[len("swift:"):])
            if not twin.strip() or not reason.strip():
                raise ValueError("API twin requires reason")
            continue
        if symbol.strip() in exceptions:
            raise ValueError("duplicate API exception: " + symbol.strip())
        exceptions[symbol.strip()] = (twin.strip(), reason.strip())
    kotlin_types = {key[1] for key, _ in kotlin if key[2] == -1}
    swift_types = {key[1] for key, _ in swift if key[2] == -1}
    for leaf, twin in aliases.items():
        if leaf not in kotlin_types or twin not in swift_types:
            raise ValueError("API twin names a missing type: " + leaf + " <-> " + twin)
    kotlin = [((aliases.get(key[0], key[0]) if not symbol.endswith(" composable") else key[0], aliases.get(key[1], key[1]) if key[2] == -1 and not symbol.endswith(" composable") else key[1], key[2]), symbol) for key, symbol in kotlin]
    result = []
    remaining = []
    paired = set()
    used = set()

    def matches(symbol, item):
        return symbol == item or symbol.startswith(item + " ") or symbol.startswith(item + ".") or symbol.startswith(item + "$") or symbol.startswith(item + "(")
    for own, other in ((swift, kotlin), (kotlin, swift)):
        retained = []
        for key, symbol in own:
            matching = [item for item in exceptions if matches(symbol, item)]
            if len(matching) > 1:
                raise ValueError("API exception is ambiguous: " + symbol)
            if matching:
                used.add(matching[0])
                twin = exceptions[matching[0]][0]
                if twin != "platform idiom" and not any(matches(item, twin) for _, item in other):
                    raise ValueError("API exception names a missing twin: " + twin)
                if twin != "platform idiom":
                    paired.add(twin)
            else:
                retained.append((key, symbol))
        remaining.append(retained)
    unused = exceptions.keys() - used
    if unused:
        raise ValueError("unused API exceptions: " + ", ".join(sorted(unused)))
    remaining = [[(key, symbol) for key, symbol in items if not any(matches(symbol, item) for item in paired)] for items in remaining]
    for own, other in ((remaining[0], remaining[1]), (remaining[1], remaining[0])):
        available = {}
        for key, _ in other:
            available[key] = available.get(key, 0) + 1
        for key, symbol in sorted(own):
            if available.get(key, 0):
                available[key] -= 1
            else:
                result.append(symbol + " has no twin for " + repr(key))
    return sorted(set(result))
