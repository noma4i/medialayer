import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
source = path.read_text()
start = source.index("fileprivate func uniffiRustCallAsync<F, T>(")
end = source.index("// Callback handlers for an async calls.", start)
template = pathlib.Path(__file__).with_name("swift_async.swift").read_text()
source = source[:start] + template + "\n" + source[end:]
source, count = re.subn(
    r"(?m)^([ \t]*)freeFunc: (ffi_\w+_rust_future_free_(\w+)),",
    lambda match: (
        match[0] + "\n" + match[1] + "cancelFunc: "
        + match[2].replace("_future_free_", "_future_cancel_") + ","
    ),
    source,
)
if count == 0 or count != source.count("await uniffiRustCallAsync("):
    raise SystemExit("Swift async call sites changed")
cancelled = 'fatalError("Cancellation not supported yet")'
if source.count(cancelled) != 1:
    raise SystemExit("Swift cancellation status changed")
source = source.replace(cancelled, "throw CancellationError()")
path.write_text(source)
