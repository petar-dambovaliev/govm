package time

type Duration int64

const (
	Nanosecond  Duration = 1
	Microsecond          = 1000 * Nanosecond
	Millisecond          = 1000 * Microsecond
	Second               = 1000 * Millisecond
	Minute               = 60 * Second
	Hour                 = 60 * Minute
)

func (d Duration) Nanoseconds() int64 {
	return int64(d)
}

func (d Duration) Microseconds() int64 {
	return int64(d) / 1000
}

func (d Duration) Milliseconds() int64 {
	return int64(d) / 1000000
}

func (d Duration) Seconds() float64 {
	sec := int64(d) / 1000000000
	nsec := int64(d) % 1000000000
	return float64(sec) + float64(nsec)/1000000000.0
}

func (d Duration) Minutes() float64 {
	min := int64(d) / 60000000000
	nsec := int64(d) % 60000000000
	return float64(min) + float64(nsec)/60000000000.0
}

func (d Duration) Hours() float64 {
	hour := int64(d) / 3600000000000
	nsec := int64(d) % 3600000000000
	return float64(hour) + float64(nsec)/3600000000000.0
}

func (d Duration) Abs() Duration {
	if d < 0 {
		return -d
	}
	return d
}

func (d Duration) Truncate(m Duration) Duration {
	if m <= 0 {
		return d
	}
	return d - d%m
}

func (d Duration) Round(m Duration) Duration {
	if m <= 0 {
		return d
	}
	r := d % m
	if d < 0 {
		r = -r
		if r+r > m {
			return d - m + r
		}
		return d + r
	}
	if r+r < m {
		return d - r
	}
	return d + m - r
}

func durationString(d Duration) string {
	if d == 0 {
		return "0s"
	}

	buf := make([]byte, 0, 32)
	u := int64(d)
	neg := d < 0
	if neg {
		u = -u
	}

	if u < 1000000000 {
		prec := 0
		suffix := "ns"
		if u >= 1000000 {
			prec = 3
			suffix = "ms"
		} else if u >= 1000 {
			prec = 3
			suffix = "\xc2\xb5s"
		}
		if prec == 0 {
			buf = appendInt(buf, u)
			i := 0
			for i < len(suffix) {
				buf = append(buf, suffix[i])
				i = i + 1
			}
		} else {
			var div int64
			if suffix == "ms" {
				div = 1000000
			} else {
				div = 1000
			}
			whole := u / div
			frac := u % div
			buf = appendInt(buf, whole)
			if frac > 0 {
				buf = append(buf, '.')
				digits := make([]byte, prec)
				i := prec - 1
				for i >= 0 {
					digits[i] = byte(frac%10) + '0'
					frac = frac / 10
					i = i - 1
				}
				last := prec - 1
				for last > 0 && digits[last] == '0' {
					last = last - 1
				}
				j := 0
				for j <= last {
					buf = append(buf, digits[j])
					j = j + 1
				}
			}
			i := 0
			for i < len(suffix) {
				buf = append(buf, suffix[i])
				i = i + 1
			}
		}
	} else {
		secs := u / 1000000000
		frac := u % 1000000000

		if secs >= 3600 {
			h := secs / 3600
			secs = secs % 3600
			buf = appendInt(buf, h)
			buf = append(buf, 'h')
		}
		if secs >= 60 || len(buf) > 0 {
			m := secs / 60
			secs = secs % 60
			if m > 0 || len(buf) > 0 {
				buf = appendInt(buf, m)
				buf = append(buf, 'm')
			}
		}
		if secs > 0 || frac > 0 || len(buf) == 0 {
			buf = appendInt(buf, secs)
			if frac > 0 {
				buf = append(buf, '.')
				digits := make([]byte, 9)
				i := 8
				for i >= 0 {
					digits[i] = byte(frac%10) + '0'
					frac = frac / 10
					i = i - 1
				}
				last := 8
				for last > 0 && digits[last] == '0' {
					last = last - 1
				}
				j := 0
				for j <= last {
					buf = append(buf, digits[j])
					j = j + 1
				}
			}
			buf = append(buf, 's')
		}
	}

	if neg {
		result := make([]byte, 0, len(buf)+1)
		result = append(result, '-')
		i := 0
		for i < len(buf) {
			result = append(result, buf[i])
			i = i + 1
		}
		return string(result)
	}
	return string(buf)
}

func (d Duration) String() string {
	return durationString(d)
}

func appendInt(buf []byte, v int64) []byte {
	if v == 0 {
		return append(buf, '0')
	}
	digits := make([]byte, 0, 20)
	for v > 0 {
		digits = append(digits, byte(v%10)+'0')
		v = v / 10
	}
	i := len(digits) - 1
	for i >= 0 {
		buf = append(buf, digits[i])
		i = i - 1
	}
	return buf
}

type Month int

const (
	January   Month = 1
	February  Month = 2
	March     Month = 3
	April     Month = 4
	May       Month = 5
	June      Month = 6
	July      Month = 7
	August    Month = 8
	September Month = 9
	October   Month = 10
	November  Month = 11
	December  Month = 12
)

func monthString(m Month) string {
	if m >= January && m <= December {
		i := int(m) - 1
		names := "JanuaryFebruaryMarchAprilMayJuneJulyAugustSeptemberOctoberNovemberDecember"
		offsets := []int{0, 7, 15, 20, 25, 28, 32, 36, 42, 51, 58, 66, 74}
		return names[offsets[i]:offsets[i+1]]
	}
	return "Month(" + intToStr(int(m)) + ")"
}

func (m Month) String() string {
	return monthString(m)
}

type Weekday int

const (
	Sunday    Weekday = 0
	Monday    Weekday = 1
	Tuesday   Weekday = 2
	Wednesday Weekday = 3
	Thursday  Weekday = 4
	Friday    Weekday = 5
	Saturday  Weekday = 6
)

func weekdayString(w Weekday) string {
	if w >= Sunday && w <= Saturday {
		names := "SundayMondayTuesdayWednesdayThursdayFridaySaturday"
		offsets := []int{0, 6, 12, 19, 28, 36, 42, 50}
		return names[offsets[int(w)]:offsets[int(w)+1]]
	}
	return "Weekday(" + intToStr(int(w)) + ")"
}

func (w Weekday) String() string {
	return weekdayString(w)
}

func intToStr(n int) string {
	if n == 0 {
		return "0"
	}
	neg := false
	if n < 0 {
		neg = true
		n = -n
	}
	digits := make([]byte, 0, 20)
	for n > 0 {
		digits = append(digits, byte(n%10)+'0')
		n = n / 10
	}
	buf := make([]byte, 0, len(digits)+1)
	if neg {
		buf = append(buf, '-')
	}
	i := len(digits) - 1
	for i >= 0 {
		buf = append(buf, digits[i])
		i = i - 1
	}
	return string(buf)
}

type Time struct {
	sec  int64
	nsec int32
}

func nowUnixNano() int64 {
	return 0
}

func Now() Time {
	n := nowUnixNano()
	return Time{sec: n / 1000000000, nsec: int32(n % 1000000000)}
}

func Since(t Time) Duration {
	now := Now()
	return now.Sub(t)
}

func Until(t Time) Duration {
	now := Now()
	return t.Sub(now)
}

func Unix(sec int64, nsec int64) Time {
	if nsec < 0 || nsec >= 1000000000 {
		sec = sec + nsec/1000000000
		nsec = nsec % 1000000000
		if nsec < 0 {
			nsec = nsec + 1000000000
			sec = sec - 1
		}
	}
	return Time{sec: sec, nsec: int32(nsec)}
}

func UnixMilli(msec int64) Time {
	return Unix(msec/1000, (msec%1000)*1000000)
}

func UnixMicro(usec int64) Time {
	return Unix(usec/1000000, (usec%1000000)*1000)
}

func (t *Time) Unix() int64 {
	return t.sec
}

func (t *Time) UnixMilli() int64 {
	return t.sec*1000 + int64(t.nsec)/1000000
}

func (t *Time) UnixMicro() int64 {
	return t.sec*1000000 + int64(t.nsec)/1000
}

func (t *Time) UnixNano() int64 {
	return t.sec*1000000000 + int64(t.nsec)
}

func (t *Time) IsZero() bool {
	return t.sec == 0 && t.nsec == 0
}

func (t *Time) Add(d Duration) Time {
	dsec := int64(d) / 1000000000
	dnsec := int64(d) % 1000000000
	nsec := int64(t.nsec) + dnsec
	sec := t.sec + dsec
	if nsec >= 1000000000 {
		sec = sec + 1
		nsec = nsec - 1000000000
	} else if nsec < 0 {
		sec = sec - 1
		nsec = nsec + 1000000000
	}
	return Time{sec: sec, nsec: int32(nsec)}
}

func (t *Time) Sub(u Time) Duration {
	d := (t.sec-u.sec)*1000000000 + int64(t.nsec) - int64(u.nsec)
	return Duration(d)
}

func (t *Time) Before(u Time) bool {
	if t.sec < u.sec {
		return true
	}
	if t.sec > u.sec {
		return false
	}
	return t.nsec < u.nsec
}

func (t *Time) After(u Time) bool {
	if t.sec > u.sec {
		return true
	}
	if t.sec < u.sec {
		return false
	}
	return t.nsec > u.nsec
}

func (t *Time) Equal(u Time) bool {
	return t.sec == u.sec && t.nsec == u.nsec
}

func (t *Time) Compare(u Time) int {
	if t.sec < u.sec {
		return -1
	}
	if t.sec > u.sec {
		return 1
	}
	if t.nsec < u.nsec {
		return -1
	}
	if t.nsec > u.nsec {
		return 1
	}
	return 0
}

const (
	secondsPerMinute = 60
	secondsPerHour   = 60 * secondsPerMinute
	secondsPerDay    = 24 * secondsPerHour
	daysPer400Years  = 365*400 + 97
	daysPer100Years  = 365*100 + 24
	daysPer4Years    = 365*4 + 1
	unixEpochDays    = 719162
)

func absDate(absSec int64) (int, Month, int, int) {
	d := absSec/secondsPerDay + unixEpochDays

	n400 := d / daysPer400Years
	d = d - n400*daysPer400Years

	n100 := d / daysPer100Years
	if n100 == 4 {
		n100 = 3
	}
	d = d - n100*daysPer100Years

	n4 := d / daysPer4Years
	d = d - n4*daysPer4Years

	n1 := d / 365
	if n1 == 4 {
		n1 = 3
	}
	d = d - n1*365

	year := int(n400*400+n100*100+n4*4+n1) + 1

	yday := int(d)
	day := yday

	if isLeapYear(year) {
		if day >= 60 {
			day = day - 1
		} else if day == 59 {
			return year, February, 29, yday + 1
		}
	}

	month := Month(day / 31)
	if month >= 12 {
		month = 11
	}
	end := daysBefore(int(month) + 1)
	if day >= end {
		month = month + 1
		if int(month) >= 12 {
			month = 11
		}
	}
	begin := daysBefore(int(month))
	month = month + 1
	day = day - begin + 1
	yday = yday + 1
	return year, month, day, yday
}

func isLeapYear(y int) bool {
	return y%4 == 0 && (y%100 != 0 || y%400 == 0)
}

func daysBefore(i int) int {
	if i == 0 {
		return 0
	} else if i == 1 {
		return 31
	} else if i == 2 {
		return 59
	} else if i == 3 {
		return 90
	} else if i == 4 {
		return 120
	} else if i == 5 {
		return 151
	} else if i == 6 {
		return 181
	} else if i == 7 {
		return 212
	} else if i == 8 {
		return 243
	} else if i == 9 {
		return 273
	} else if i == 10 {
		return 304
	} else if i == 11 {
		return 334
	} else if i == 12 {
		return 365
	}
	return 0
}

func daysInMonth(year int, m Month) int {
	if m == February && isLeapYear(year) {
		return 29
	}
	return daysBefore(int(m)) - daysBefore(int(m)-1)
}

func (t *Time) Year() int {
	year, _, _, _ := absDate(t.sec)
	return year
}

func (t *Time) Month() Month {
	_, month, _, _ := absDate(t.sec)
	return month
}

func (t *Time) Day() int {
	_, _, day, _ := absDate(t.sec)
	return day
}

func (t *Time) YearDay() int {
	_, _, _, yday := absDate(t.sec)
	return yday
}

func (t *Time) Hour() int {
	return int((t.sec % secondsPerDay) / secondsPerHour)
}

func (t *Time) Minute() int {
	return int((t.sec % secondsPerHour) / secondsPerMinute)
}

func (t *Time) Second() int {
	return int(t.sec % secondsPerMinute)
}

func (t *Time) Nanosecond() int {
	return int(t.nsec)
}

func (t *Time) Weekday() Weekday {
	d := (t.sec/secondsPerDay + int64(unixEpochDays)) % 7
	if d < 0 {
		d = d + 7
	}
	wd := int((d + 1) % 7)
	return Weekday(wd)
}

func (t *Time) Date() (int, Month, int) {
	year, month, day, _ := absDate(t.sec)
	return year, month, day
}

func (t *Time) Clock() (int, int, int) {
	return t.Hour(), t.Minute(), t.Second()
}

func (t *Time) AddDate(years int, months int, days int) Time {
	year, month, day, _ := absDate(t.sec)
	year = year + years
	mi := int(month) - 1 + months
	if mi >= 12 {
		year = year + mi/12
		mi = mi % 12
	} else if mi < 0 {
		year = year + (mi-11)/12
		mi = mi%12 + 12
		if mi >= 12 {
			mi = mi - 12
		}
	}
	month = Month(mi + 1)

	maxDay := daysInMonth(year, month)
	if day > maxDay {
		day = maxDay
	}

	return DateFull(year, month, day+days, t.Hour(), t.Minute(), t.Second(), int(t.nsec))
}

func DateFull(year int, month Month, day int, hour int, min int, sec int, nsec int) Time {
	mi := int(month) - 1
	if mi >= 12 {
		year = year + mi/12
		mi = mi % 12
	} else if mi < 0 {
		year = year + (mi-11)/12
		mi = mi%12 + 12
		if mi >= 12 {
			mi = mi - 12
		}
	}
	month = Month(mi + 1)

	y := int64(year) - 1
	d := y*365 + y/4 - y/100 + y/400 - int64(unixEpochDays)
	d = d + int64(daysBefore(int(month)-1))
	if isLeapYear(year) && month > February {
		d = d + 1
	}
	d = d + int64(day) - 1

	absSec := d*secondsPerDay + int64(hour)*int64(secondsPerHour) + int64(min)*int64(secondsPerMinute) + int64(sec)

	if nsec < 0 || nsec >= 1000000000 {
		absSec = absSec + int64(nsec)/1000000000
		nsec = nsec % 1000000000
		if nsec < 0 {
			nsec = nsec + 1000000000
			absSec = absSec - 1
		}
	}

	return Time{sec: absSec, nsec: int32(nsec)}
}

type Location struct {
	name   string
	offset int
}

func getUTC() *Location {
	return &Location{name: "UTC", offset: 0}
}

func FixedZone(name string, offset int) *Location {
	return &Location{name: name, offset: offset}
}

func (l *Location) String() string {
	return l.name
}

func (t *Time) Zone() (string, int) {
	return "UTC", 0
}

func (t *Time) UTC() Time {
	return *t
}

func (t *Time) Local() Time {
	return *t
}

func (t *Time) In(loc *Location) Time {
	return *t
}

const (
	RFC3339     = "2006-01-02T15:04:05Z07:00"
	RFC3339Nano = "2006-01-02T15:04:05.999999999Z07:00"
	DateTime    = "2006-01-02 15:04:05"
	DateOnly    = "2006-01-02"
	TimeOnly    = "15:04:05"
)

func padTwo(buf []byte, v int) []byte {
	if v < 10 {
		buf = append(buf, '0')
	}
	return appendIntBuf(buf, v)
}

func padFour(buf []byte, v int) []byte {
	if v < 10 {
		buf = append(buf, '0', '0', '0')
	} else if v < 100 {
		buf = append(buf, '0', '0')
	} else if v < 1000 {
		buf = append(buf, '0')
	}
	return appendIntBuf(buf, v)
}

func appendIntBuf(buf []byte, v int) []byte {
	if v == 0 {
		return append(buf, '0')
	}
	neg := false
	if v < 0 {
		neg = true
		v = -v
	}
	digits := make([]byte, 0, 10)
	for v > 0 {
		digits = append(digits, byte(v%10)+'0')
		v = v / 10
	}
	if neg {
		buf = append(buf, '-')
	}
	i := len(digits) - 1
	for i >= 0 {
		buf = append(buf, digits[i])
		i = i - 1
	}
	return buf
}

func formatNano(buf []byte, ns int, digits int) []byte {
	buf = append(buf, '.')
	d := make([]byte, 9)
	i := 8
	for i >= 0 {
		d[i] = byte(ns%10) + '0'
		ns = ns / 10
		i = i - 1
	}
	j := 0
	for j < digits {
		buf = append(buf, d[j])
		j = j + 1
	}
	return buf
}

func (t *Time) formatRFC3339(buf []byte, nano bool) []byte {
	year, month, day, _ := absDate(t.sec)
	buf = padFour(buf, year)
	buf = append(buf, '-')
	buf = padTwo(buf, int(month))
	buf = append(buf, '-')
	buf = padTwo(buf, day)
	buf = append(buf, 'T')
	buf = padTwo(buf, t.Hour())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Minute())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Second())
	if nano && t.nsec != 0 {
		buf = formatNano(buf, int(t.nsec), 9)
	}
	buf = append(buf, 'Z')
	return buf
}

func (t *Time) formatDateTime(buf []byte) []byte {
	year, month, day, _ := absDate(t.sec)
	buf = padFour(buf, year)
	buf = append(buf, '-')
	buf = padTwo(buf, int(month))
	buf = append(buf, '-')
	buf = padTwo(buf, day)
	buf = append(buf, ' ')
	buf = padTwo(buf, t.Hour())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Minute())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Second())
	return buf
}

func (t *Time) formatDateOnly(buf []byte) []byte {
	year, month, day, _ := absDate(t.sec)
	buf = padFour(buf, year)
	buf = append(buf, '-')
	buf = padTwo(buf, int(month))
	buf = append(buf, '-')
	buf = padTwo(buf, day)
	return buf
}

func (t *Time) formatTimeOnly(buf []byte) []byte {
	buf = padTwo(buf, t.Hour())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Minute())
	buf = append(buf, ':')
	buf = padTwo(buf, t.Second())
	return buf
}

func (t *Time) Format(layout string) string {
	buf := make([]byte, 0, 64)
	if layout == RFC3339 {
		buf = t.formatRFC3339(buf, false)
	} else if layout == RFC3339Nano {
		buf = t.formatRFC3339(buf, true)
	} else if layout == DateTime {
		buf = t.formatDateTime(buf)
	} else if layout == DateOnly {
		buf = t.formatDateOnly(buf)
	} else if layout == TimeOnly {
		buf = t.formatTimeOnly(buf)
	} else {
		buf = t.formatDateTime(buf)
	}
	return string(buf)
}

