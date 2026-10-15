# IRC parser tests

These fixtures come from <https://github.com/ircdocs/parser-tests> at commit
`6b417e666de20ba677b14e0189213b3706009df6` and are distributed under the
included CC0 licence.

They were converted from YAML with `yq` 3.4.3:

```sh
for name in msg-split msg-join userhost-split validate-hostname; do
  yq . "tests/$name.yaml" > "$name.json"
done
```
