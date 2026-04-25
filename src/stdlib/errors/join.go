package errors

type joinError struct {
	msg string
}

func (e *joinError) Error() string {
	return e.msg
}

func Join(errs ...error) error {
	n := 0
	for _, err := range errs {
		if err != nil {
			n++
		}
	}
	if n == 0 {
		return nil
	}
	return &joinError{msg: "multiple errors"}
}
