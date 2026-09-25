"""Load PNML files as place/transition nets with pm4py."""
import contextlib
import io
import sys
import warnings

warnings.filterwarnings("ignore")
with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
    import pm4py

failed = 0
for path in sys.argv[1:]:
    try:
        net, marking, _ = pm4py.read_pnml(path)
        assert net.places and net.transitions, "empty net"
    except Exception as error:  # report every file, not only the first
        failed += 1
        print("FAIL", path, error)
print(f"pnml: {len(sys.argv) - 1 - failed} ok, {failed} failed")
sys.exit(1 if failed else 0)
