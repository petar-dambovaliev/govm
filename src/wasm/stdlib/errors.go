package errors

type errorString struct {
	s string
}

func (e *errorString) Error() string {
	return e.s
}

func New(text string) error {
	return &errorString{s: text}
}

var ErrUnsupported = New("unsupported operation")

// Requires type assertions against anonymous interfaces -- not yet implemented in the compiler
// func Unwrap(err error) error { ... }

// Requires type assertions against anonymous interfaces + type switches -- not yet implemented in the compiler
// func Is(err, target error) bool { ... }
// func is(err, target error) bool { ... }

// Requires type assertions against anonymous interfaces -- not yet implemented in the compiler
// type joinError struct { errs []error }
// func (e *joinError) Error() string { ... }
// func (e *joinError) Unwrap() []error { ... }
// func Join(errs ...error) error { ... }

// Requires reflection (internal/reflectlite) -- not yet implemented in the compiler
// func As(err error, target any) bool { ... }

// Requires reflection + generics with reflection internals -- not yet implemented in the compiler
// func AsType[E error](err error) (E, bool) { ... }
