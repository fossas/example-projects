## Example maven project with reachable vulnerabilities

- To build project: `mvn package` (you will need java8+, and maven)

```bash
; mvn package # build project
; fossa analyze -o --debug # run fossa analysis in output mode only
; fossa analyze --debug -p example-maven-vuln-function-used -r 1 # run fossa analysis
```