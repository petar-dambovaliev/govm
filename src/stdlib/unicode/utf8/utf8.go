package utf8

const (
	RuneError = 0xFFFD
	RuneSelf  = 0x80
	MaxRune   = 0x10FFFF
	UTFMax    = 4
)

const (
	surrogateMin = 0xD800
	surrogateMax = 0xDFFF
)

const (
	tx = 0x80
	t2 = 0xC0
	t3 = 0xE0
	t4 = 0xF0

	maskx = 0x3F
	mask2 = 0x1F
	mask3 = 0x0F
	mask4 = 0x07

	rune1Max = 1<<7 - 1
	rune2Max = 1<<11 - 1
	rune3Max = 1<<16 - 1

	locb = 0x80
	hicb = 0xBF
)

func byteClass(b int) int {
	if b < 0x80 {
		return 0x10
	}
	if b < 0xC2 {
		return 0x01
	}
	if b < 0xE0 {
		return 0x02
	}
	if b == 0xE0 {
		return 0x13
	}
	if b < 0xED {
		return 0x03
	}
	if b == 0xED {
		return 0x23
	}
	if b < 0xF0 {
		return 0x03
	}
	if b == 0xF0 {
		return 0x34
	}
	if b < 0xF4 {
		return 0x04
	}
	if b == 0xF4 {
		return 0x44
	}
	return 0x01
}

func acceptLo(idx int) int {
	if idx == 1 {
		return 0xA0
	}
	if idx == 3 {
		return 0x90
	}
	return locb
}

func acceptHi(idx int) int {
	if idx == 2 {
		return 0x9F
	}
	if idx == 4 {
		return 0x8F
	}
	return hicb
}

func FullRuneInString(s string) bool {
	n := len(s)
	if n == 0 {
		return false
	}
	x := byteClass(int(s[0]))
	sz := x & 0x0F
	if n >= sz {
		return true
	}
	aIdx := x >> 4
	if n > 1 {
		b1 := int(s[1])
		if b1 < acceptLo(aIdx) || acceptHi(aIdx) < b1 {
			return true
		}
	}
	if n > 2 {
		b2 := int(s[2])
		if b2 < locb || hicb < b2 {
			return true
		}
	}
	return false
}

func DecodeRuneInString(s string) (rune, int) {
	n := len(s)
	if n < 1 {
		return RuneError, 0
	}
	s0 := int(s[0])
	if s0 < RuneSelf {
		return rune(s0), 1
	}
	x := byteClass(s0)
	sz := x & 0x0F
	if sz <= 1 {
		return RuneError, 1
	}
	aIdx := x >> 4
	if n < sz {
		return RuneError, 1
	}
	b1 := int(s[1])
	lo := acceptLo(aIdx)
	hi := acceptHi(aIdx)
	if b1 < lo || hi < b1 {
		return RuneError, 1
	}
	if sz == 2 {
		r := (s0&mask2)<<6 | (b1 & maskx)
		return rune(r), 2
	}
	b2 := int(s[2])
	if b2 < locb || hicb < b2 {
		return RuneError, 1
	}
	if sz == 3 {
		r := (s0&mask3)<<12 | (b1&maskx)<<6 | (b2 & maskx)
		return rune(r), 3
	}
	b3 := int(s[3])
	if b3 < locb || hicb < b3 {
		return RuneError, 1
	}
	r := (s0&mask4)<<18 | (b1&maskx)<<12 | (b2&maskx)<<6 | (b3 & maskx)
	return rune(r), 4
}

func DecodeLastRuneInString(s string) (rune, int) {
	end := len(s)
	if end == 0 {
		return RuneError, 0
	}
	start := end - 1
	r := rune(s[start])
	if r < RuneSelf {
		return r, 1
	}
	lim := end - UTFMax
	if lim < 0 {
		lim = 0
	}
	for start = start - 1; start >= lim; start-- {
		if RuneStart(s[start]) {
			break
		}
	}
	if start < 0 {
		start = 0
	}
	r, size := DecodeRuneInString(s[start:end])
	if start+size != end {
		return RuneError, 1
	}
	return r, size
}

func RuneLen(r rune) int {
	if r < 0 {
		return -1
	}
	if r <= rune1Max {
		return 1
	}
	if r <= rune2Max {
		return 2
	}
	if surrogateMin <= r && r <= surrogateMax {
		return -1
	}
	if r <= rune3Max {
		return 3
	}
	if r <= MaxRune {
		return 4
	}
	return -1
}

func RuneCountInString(s string) int {
	n := 0
	for range s {
		n++
	}
	return n
}

func RuneStart(b byte) bool {
	return int(b)&0xC0 != 0x80
}

func ValidString(s string) bool {
	n := len(s)
	i := 0
	for i < n {
		si := int(s[i])
		if si < RuneSelf {
			i = i + 1
			continue
		}
		x := byteClass(si)
		sz := x & 0x0F
		if sz <= 1 {
			return false
		}
		aIdx := x >> 4
		if i+sz > n {
			return false
		}
		b1 := int(s[i+1])
		if b1 < acceptLo(aIdx) || acceptHi(aIdx) < b1 {
			return false
		}
		if sz == 2 {
			i = i + 2
			continue
		}
		b2 := int(s[i+2])
		if b2 < locb || hicb < b2 {
			return false
		}
		if sz == 3 {
			i = i + 3
			continue
		}
		b3 := int(s[i+3])
		if b3 < locb || hicb < b3 {
			return false
		}
		i = i + 4
	}
	return true
}

func ValidRune(r rune) bool {
	if 0 <= r && r < surrogateMin {
		return true
	}
	if surrogateMax < r && r <= MaxRune {
		return true
	}
	return false
}

func FullRune(p []byte) bool {
	n := len(p)
	if n == 0 {
		return false
	}
	x := byteClass(int(p[0]))
	sz := x & 0x0F
	if n >= sz {
		return true
	}
	aIdx := x >> 4
	if n > 1 {
		b1 := int(p[1])
		if b1 < acceptLo(aIdx) || acceptHi(aIdx) < b1 {
			return true
		}
	}
	if n > 2 {
		b2 := int(p[2])
		if b2 < locb || hicb < b2 {
			return true
		}
	}
	return false
}

func DecodeRune(p []byte) (rune, int) {
	n := len(p)
	if n < 1 {
		return RuneError, 0
	}
	p0 := int(p[0])
	if p0 < RuneSelf {
		return rune(p0), 1
	}
	x := byteClass(p0)
	sz := x & 0x0F
	if sz <= 1 {
		return RuneError, 1
	}
	aIdx := x >> 4
	if n < sz {
		return RuneError, 1
	}
	b1 := int(p[1])
	lo := acceptLo(aIdx)
	hi := acceptHi(aIdx)
	if b1 < lo || hi < b1 {
		return RuneError, 1
	}
	if sz == 2 {
		r := (p0&mask2)<<6 | (b1 & maskx)
		return rune(r), 2
	}
	b2 := int(p[2])
	if b2 < locb || hicb < b2 {
		return RuneError, 1
	}
	if sz == 3 {
		r := (p0&mask3)<<12 | (b1&maskx)<<6 | (b2 & maskx)
		return rune(r), 3
	}
	b3 := int(p[3])
	if b3 < locb || hicb < b3 {
		return RuneError, 1
	}
	r := (p0&mask4)<<18 | (b1&maskx)<<12 | (b2&maskx)<<6 | (b3 & maskx)
	return rune(r), 4
}

func DecodeLastRune(p []byte) (rune, int) {
	end := len(p)
	if end == 0 {
		return RuneError, 0
	}
	start := end - 1
	r := rune(p[start])
	if r < RuneSelf {
		return r, 1
	}
	lim := end - UTFMax
	if lim < 0 {
		lim = 0
	}
	for start = start - 1; start >= lim; start-- {
		if RuneStart(p[start]) {
			break
		}
	}
	if start < 0 {
		start = 0
	}
	r, size := DecodeRune(p[start:end])
	if start+size != end {
		return RuneError, 1
	}
	return r, size
}

func EncodeRune(p []byte, r rune) int {
	i := uint32(r)
	if i <= uint32(rune1Max) {
		p[0] = byte(r)
		return 1
	}
	if i <= uint32(rune2Max) {
		p[0] = byte(int(t2) | (int(r) >> 6))
		p[1] = byte(int(tx) | (int(r) & maskx))
		return 2
	}
	if i < uint32(surrogateMin) || (uint32(surrogateMax) < i && i <= uint32(rune3Max)) {
		p[0] = byte(int(t3) | (int(r) >> 12))
		p[1] = byte(int(tx) | ((int(r) >> 6) & maskx))
		p[2] = byte(int(tx) | (int(r) & maskx))
		return 3
	}
	if i > uint32(rune3Max) && i <= uint32(MaxRune) {
		p[0] = byte(int(t4) | (int(r) >> 18))
		p[1] = byte(int(tx) | ((int(r) >> 12) & maskx))
		p[2] = byte(int(tx) | ((int(r) >> 6) & maskx))
		p[3] = byte(int(tx) | (int(r) & maskx))
		return 4
	}
	p[0] = 0xEF
	p[1] = 0xBF
	p[2] = 0xBD
	return 3
}

func AppendRune(p []byte, r rune) []byte {
	buf := make([]byte, 4)
	n := EncodeRune(buf, r)
	j := 0
	for j < n {
		p = append(p, buf[j])
		j = j + 1
	}
	return p
}

func RuneCount(p []byte) int {
	np := len(p)
	n := 0
	i := 0
	for i < np {
		b := int(p[i])
		if b < RuneSelf {
			i = i + 1
			n = n + 1
			continue
		}
		x := byteClass(b)
		sz := x & 0x0F
		if sz <= 1 || i+sz > np {
			i = i + 1
			n = n + 1
			continue
		}
		aIdx := x >> 4
		b1 := int(p[i+1])
		if b1 < acceptLo(aIdx) || acceptHi(aIdx) < b1 {
			i = i + 1
			n = n + 1
			continue
		}
		if sz == 2 {
			i = i + 2
			n = n + 1
			continue
		}
		b2 := int(p[i+2])
		if b2 < locb || hicb < b2 {
			i = i + 1
			n = n + 1
			continue
		}
		if sz == 3 {
			i = i + 3
			n = n + 1
			continue
		}
		b3 := int(p[i+3])
		if b3 < locb || hicb < b3 {
			i = i + 1
			n = n + 1
			continue
		}
		i = i + 4
		n = n + 1
	}
	return n
}

func Valid(p []byte) bool {
	n := len(p)
	i := 0
	for i < n {
		pi := int(p[i])
		if pi < RuneSelf {
			i = i + 1
			continue
		}
		x := byteClass(pi)
		sz := x & 0x0F
		if sz <= 1 {
			return false
		}
		aIdx := x >> 4
		if i+sz > n {
			return false
		}
		b1 := int(p[i+1])
		if b1 < acceptLo(aIdx) || acceptHi(aIdx) < b1 {
			return false
		}
		if sz == 2 {
			i = i + 2
			continue
		}
		b2 := int(p[i+2])
		if b2 < locb || hicb < b2 {
			return false
		}
		if sz == 3 {
			i = i + 3
			continue
		}
		b3 := int(p[i+3])
		if b3 < locb || hicb < b3 {
			return false
		}
		i = i + 4
	}
	return true
}
