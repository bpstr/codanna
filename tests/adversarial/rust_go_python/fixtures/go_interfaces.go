package fixtures

type Reader interface { Read([]byte) (int, error) }
type WrongSignature struct{}
func (WrongSignature) Read(string) string { return "" }

type Socket struct{}
func (*Socket) Read([]byte) (int, error) { return 0, nil }

type Transport interface { Reader }
type Wrapper struct { *Socket }

var _ Reader = (*Socket)(nil)
var _ Reader = Wrapper{}

// Expected: *Socket and Wrapper satisfy Reader. Socket (value) and
// WrongSignature do not. Both embedding relations must be retained.
