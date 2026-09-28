# Security

Waffle opens files from untrusted sources, so parsing bugs can matter.

- It never executes macros. `vbaProject.bin` is carried through saves untouched.
- It makes no network requests.

If you find a crash or memory-safety issue triggered by a crafted file, please report it privately through GitHub's "Report a vulnerability" (Security tab) rather than in a public issue. Include the file if you can. We'll acknowledge within a few days.
