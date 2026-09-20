package fixtures

func Identity[T any](value T) T { return value }
func Pair[T any](first, second T) T { return first }
func Empty[T any]() {}
func TwoTypes[T any, U any](first T, second U) T { return first }
type Number[T any] int

func GenericCalls() {
    Identity[int](1)
    Pair[int](1, 2)
    Empty[int]()
    TwoTypes[int, string](1, "two")
    // Real conversion: must never become a call to a same-named function.
    _ = Number[int](1)
}
