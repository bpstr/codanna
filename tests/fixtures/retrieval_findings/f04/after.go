package fixture
// A new line moves the original declaration.
func DecodeFrame(input []byte) bool {
 return validateFrame(input)
}
func validateFrame(input []byte) bool { return len(input) > 1 }
