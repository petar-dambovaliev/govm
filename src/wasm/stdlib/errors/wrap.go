package errors

func Unwrap(err error) error {
	u, ok := err.(interface{ Unwrap() error })
	if !ok {
		return nil
	}
	return u.Unwrap()
}

func Is(err, target error) bool {
	if err == target {
		return true
	}
	return is(err, target)
}

func is(err, target error) bool {
	for {
		if err == target {
			return true
		}
		u, ok := err.(interface{ Unwrap() error })
		if !ok {
			return false
		}
		err = u.Unwrap()
	}
}

// Requires reflection (internal/reflectlite) -- not yet implemented in the compiler
// func As(err error, target any) bool { ... }
