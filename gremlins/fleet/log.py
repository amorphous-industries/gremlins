"""Log subcommand."""

import os
import sys

from gremlins.fleet.resolve import resolve_gremlin


def do_log(target: str, *, full: bool = False) -> bool:
    """Tail (default) or dump (--full) the gremlin's log file. Execs ``tail -F``
    or ``cat`` so Ctrl-C handling and rotation/truncation behavior are whatever
    the tool provides — the wrapper just resolves the id and prints the path."""
    match = resolve_gremlin(target)
    if match is None:
        return False

    gremlin_id, _, wdir = match
    log_path = os.path.join(wdir, "log")
    if not os.path.isfile(log_path):
        sys.stderr.write(f"error: no log file for gremlin {gremlin_id} at {log_path}\n")
        return False

    if full:
        try:
            os.execvp("cat", ["cat", log_path])
        except FileNotFoundError:
            sys.stderr.write("error: cat not found in PATH\n")
            return False
        except OSError as e:
            sys.stderr.write(f"error: could not exec cat: {e}\n")
            return False

    # Print the path to stderr so it survives even if the operator is piping
    # tail's stdout into another tool. Flush immediately so the header isn't
    # interleaved after tail starts writing.
    sys.stderr.write(f"==> log: {log_path}\n")
    sys.stderr.flush()

    try:
        os.execvp("tail", ["tail", "-F", log_path])
    except FileNotFoundError:
        sys.stderr.write("error: tail not found in PATH\n")
        return False
    except OSError as e:
        sys.stderr.write(f"error: could not exec tail: {e}\n")
        return False
