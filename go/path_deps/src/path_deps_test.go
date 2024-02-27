package main

import (
	"os"
	"testing"

	"github.com/ajankovic/xdiff"
	"github.com/ajankovic/xdiff/parser"
)

var testXmlA = `
<note>
<to>Tove</to>
<from>Jani</from>
<heading>Reminder</heading>
<body>Don't forget me this weekend!</body>
</note>
`

var testXmlB = `
<note>
<to>Jani</to>
<from>Tove</from>
<heading>Response</heading>
<body>I won't!</body>
</note>
`

func AssertXMLEquals(t *testing.T, expected, actual string) {
	p := parser.New()
	left, _ := p.ParseBytes([]byte(expected))
	right, _ := p.ParseBytes([]byte(actual))

	diff, _ := xdiff.Compare(left, right)
	if diff != nil {
		enc := xdiff.NewTextEncoder(os.Stderr)
		enc.Encode(diff)
		t.Fatalf("\ndiff: %s \nactual:%s\nexpected: %s\n", diff, actual, expected)
	}
}
func TestXdiff(t *testing.T) {
	AssertXMLEquals(t, testXmlA, testXmlB)
}
