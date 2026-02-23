package time

func (t Time) appendFormatRFC3339(b []byte, nanos bool) []byte {
	_, offset, abs := t.locabs()
	year, month, day := abs.days().date()
	b = appendInt(b, year, 4)
	b = append(b, '-')
	b = appendInt(b, int(month), 2)
	b = append(b, '-')
	b = appendInt(b, day, 2)
	b = append(b, 'T')
	hour, min, sec := abs.clock()
	b = appendInt(b, hour, 2)
	b = append(b, ':')
	b = appendInt(b, min, 2)
	b = append(b, ':')
	b = appendInt(b, sec, 2)
	if nanos {
		std := stdFracSecond(stdFracSecond9, 9, '.')
		b = appendNano(b, t.Nanosecond(), std)
	}
	if offset == 0 {
		return append(b, 'Z')
	}
	zone := offset / 60
	if zone < 0 {
		b = append(b, '-')
		zone = -zone
	} else {
		b = append(b, '+')
	}
	b = appendInt(b, zone/60, 2)
	b = append(b, ':')
	b = appendInt(b, zone%60, 2)
	return b
}

func (t Time) appendStrictRFC3339(b []byte) ([]byte, error) {
	b = t.appendFormatRFC3339(b, true)
	return b, nil
}

func rfc3339ParseUint(s string, min int, max int) (int, bool) {
	x := 0
	for i := 0; i < len(s); i++ {
		c := s[i]
		if c < '0' || '9' < c {
			return min, false
		}
		x = x*10 + int(c) - '0'
	}
	if x < min || max < x {
		return min, false
	}
	return x, true
}

func parseRFC3339(s string, local *Location) (Time, bool) {
	if len(s) < len("2006-01-02T15:04:05") {
		return Time{}, false
	}
	year, ok1 := rfc3339ParseUint(s[0:4], 0, 9999)
	month, ok2 := rfc3339ParseUint(s[5:7], 1, 12)
	day, ok3 := rfc3339ParseUint(s[8:10], 1, daysIn(Month(month), year))
	hour, ok4 := rfc3339ParseUint(s[11:13], 0, 23)
	min, ok5 := rfc3339ParseUint(s[14:16], 0, 59)
	sec, ok6 := rfc3339ParseUint(s[17:19], 0, 59)
	if !(ok1 && ok2 && ok3 && ok4 && ok5 && ok6) {
		return Time{}, false
	}
	if !(s[4] == '-' && s[7] == '-' && s[10] == 'T' && s[13] == ':' && s[16] == ':') {
		return Time{}, false
	}
	s = s[19:]
	var nsec int
	if len(s) >= 2 && s[0] == '.' && isDigit(s, 1) {
		n := 2
		for ; n < len(s) && isDigit(s, n); n++ {
		}
		nsec, _, _ = parseNanoseconds(s, n)
		s = s[n:]
	}
	t := Date(year, Month(month), day, hour, min, sec, nsec, UTC)
	if len(s) != 1 || s[0] != 'Z' {
		if len(s) != len("-07:00") {
			return Time{}, false
		}
		hr, ok7 := rfc3339ParseUint(s[1:3], 0, 23)
		mm, ok8 := rfc3339ParseUint(s[4:6], 0, 59)
		if !(ok7 && ok8) || !((s[0] == '-' || s[0] == '+') && s[3] == ':') {
			return Time{}, false
		}
		zoneOffset := (hr*60 + mm) * 60
		if s[0] == '-' {
			zoneOffset = zoneOffset * -1
		}
		t.addSec(-int64(zoneOffset))
		if _, offset, _, _, _ := local.lookup(t.unixSec()); offset == zoneOffset {
			t.setLoc(local)
		} else {
			t.setLoc(FixedZone("", zoneOffset))
		}
	}
	return t, true
}

func parseStrictRFC3339(b []byte) (Time, error) {
	t, ok := parseRFC3339(string(b), Local)
	if !ok {
		t2, err := Parse(RFC3339, string(b))
		if err != nil {
			return Time{}, err
		}
		return t2, nil
	}
	return t, nil
}
