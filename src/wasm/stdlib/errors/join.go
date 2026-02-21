package errors

// Requires type assertions against anonymous interfaces -- not yet implemented in the compiler

// type joinError struct {
// 	errs []error
// }

// func (e *joinError) Error() string {
// 	// TODO: implement string building with loop
// 	return "multiple errors"
// }

// func (e *joinError) Unwrap() []error {
// 	return e.errs
// }

// func Join(errs ...error) error {
// 	var nonNil []error
// 	for _, err := range errs {
// 		if err != nil {
// 			nonNil = append(nonNil, err)
// 		}
// 	}
// 	if len(nonNil) == 0 {
// 		return nil
// 	}
// 	return &joinError{errs: nonNil}
// }
