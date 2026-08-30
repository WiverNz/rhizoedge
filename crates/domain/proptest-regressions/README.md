# Regression corpus

`proptest` writes a shrunk counterexample here the first time a property test
fails, and replays every recorded case before generating new ones on every run
afterwards.

**This directory is committed on purpose.** A found bug that stops being tested
is a bug that comes back, and a counterexample is the cheapest permanent
evidence there is: one line, replayed in microseconds, for ever.

Do not delete an entry to make a suite green. The entry is the bug report.
