# known issues
there are some known edge cases where the tool might have unexpected behavior

### none persistent chat
this is not supported by the `what-cli` tool, might be supported in the future if
people will need it

### panic when piped program expects input
this happens when we pipe a command output into `what`, but the piped program
expects some input, this is known issue

reacreate the issue:
```sh
python3 -c "input('blocking>')" | what
```
