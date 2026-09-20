package fixtures

import "bytes"

var _ = bytes.MinRead

type Codec struct{}
func (c *Codec) Reset() {}

func shadowedPackage(bytes *Codec) {
    bytes.Reset()
}

func groupedParameters(first, second *Codec) {
    first.Reset()
    second.Reset()
}

type Box[T any] struct{}
func (b Box[T]) Reset() {}

func genericLiteral() {
    box := Box[int]{}
    box.Reset()
}

func inferredVar() {
    var codec = Codec{}
    codec.Reset()
}

func loadConfig() int { return 1 }
var GlobalConfig = loadConfig()
