-- md5.lua — RFC 1321 MD5 in pure Lua 5.4.
--
-- Used by the bundled `geohash` plugin (xkcd #426 geohashing
-- algorithm, which feeds today's date + DJIA opening value through
-- MD5 to derive a daily destination coordinate). Pure Lua because
-- the Rust side deliberately exposes no crypto and the geohash
-- plugin is the only consumer — keeping it next to the plugin makes
-- the dependency obvious and avoids adding a runtime-wide crypto
-- surface for a single use case.
--
-- Lua 5.4's native bitwise operators (`&` `|` `~` `<<` `>>`) make
-- the implementation a direct transliteration of the spec; no
-- third-party shims, no metatable trickery. Integers are 64-bit so
-- intermediate values can carry stray high bits, but every spot
-- that feeds back into a 32-bit register is masked with
-- `& 0xffffffff`, and `string.pack("<I4", ...)` does the final
-- 32-bit truncation when emitting the digest bytes.
--
-- Verified against the RFC 1321 test vectors plus a few extras (see
-- `_TEST_VECTORS` at the bottom — run with `lua5.4 md5.lua` to
-- self-check).

local M = {}

-- Per-round left-shift amounts (RFC 1321 §3.4).
local S = {
     7, 12, 17, 22,  7, 12, 17, 22,  7, 12, 17, 22,  7, 12, 17, 22,
     5,  9, 14, 20,  5,  9, 14, 20,  5,  9, 14, 20,  5,  9, 14, 20,
     4, 11, 16, 23,  4, 11, 16, 23,  4, 11, 16, 23,  4, 11, 16, 23,
     6, 10, 15, 21,  6, 10, 15, 21,  6, 10, 15, 21,  6, 10, 15, 21,
}

-- T[i] = floor(abs(sin(i)) * 2^32) for i = 1..64. Pre-computed at
-- module load so each digest is a straight transform.
local T = {}
for i = 1, 64 do
    T[i] = math.floor(math.abs(math.sin(i)) * 4294967296) & 0xffffffff
end

local function leftrotate(x, n)
    return ((x << n) | (x >> (32 - n))) & 0xffffffff
end

--- Compute the 32-character lowercase hex MD5 digest of `input`.
function M.hex(input)
    -- Padding: append 0x80, then zeros so the post-pad length is
    -- ≡ 56 (mod 64), then the original message length in bits as a
    -- little-endian 64-bit integer.
    local input_len = #input
    local pad_zeros = (56 - input_len - 1) % 64
    local data = input
        .. "\x80"
        .. string.rep("\0", pad_zeros)
        .. string.pack("<I8", input_len * 8)

    local a0, b0, c0, d0 = 0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476

    for off = 1, #data, 64 do
        local W = {}
        for j = 1, 16 do
            W[j] = string.unpack("<I4", data, off + (j - 1) * 4)
        end

        local a, b, c, d = a0, b0, c0, d0
        for i = 1, 64 do
            local f, g
            if i <= 16 then
                f = (b & c) | ((~b) & d)
                g = i
            elseif i <= 32 then
                f = (d & b) | ((~d) & c)
                g = ((5 * (i - 1) + 1) % 16) + 1
            elseif i <= 48 then
                f = b ~ c ~ d
                g = ((3 * (i - 1) + 5) % 16) + 1
            else
                f = c ~ (b | (~d))
                g = ((7 * (i - 1)) % 16) + 1
            end
            f = (f + a + T[i] + W[g]) & 0xffffffff
            a, d, c, b = d, c, b, (b + leftrotate(f, S[i])) & 0xffffffff
        end
        a0 = (a0 + a) & 0xffffffff
        b0 = (b0 + b) & 0xffffffff
        c0 = (c0 + c) & 0xffffffff
        d0 = (d0 + d) & 0xffffffff
    end

    -- The digest is the four state words emitted as little-endian
    -- bytes. `string.pack("<I4")` takes care of the byte order;
    -- gsub turns the 16 raw bytes into 32 hex characters.
    local raw = string.pack("<I4I4I4I4", a0, b0, c0, d0)
    return (raw:gsub(".", function(c) return string.format("%02x", string.byte(c)) end))
end

-- RFC 1321 test vectors — kept inline so `lua5.4 md5.lua` self-tests.
M._TEST_VECTORS = {
    { ""                                                                , "d41d8cd98f00b204e9800998ecf8427e" },
    { "a"                                                               , "0cc175b9c0f1b6a831c399e269772661" },
    { "abc"                                                             , "900150983cd24fb0d6963f7d28e17f72" },
    { "message digest"                                                  , "f96b697d7cb7938d525a2f31aaf161d0" },
    { "abcdefghijklmnopqrstuvwxyz"                                      , "c3fcd3d76192e4007dfb496cca67e13b" },
    { "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"  , "d174ab98d277d9f5a5611c2c9f419d9f" },
    { string.rep("1234567890", 8)                                       , "57edf4a22be3c955ac49da2e2107b67a" },
    { "The quick brown fox jumps over the lazy dog"                     , "9e107d9d372bb6826bd81d3542a419d6" },
}

if arg and arg[0] and arg[0]:match("md5%.lua$") then
    local fail = 0
    for _, t in ipairs(M._TEST_VECTORS) do
        local got = M.hex(t[1])
        if got ~= t[2] then
            fail = fail + 1
            print(string.format("FAIL md5(%q) = %s, want %s", t[1], got, t[2]))
        end
    end
    if fail == 0 then
        print(string.format("OK %d/%d", #M._TEST_VECTORS, #M._TEST_VECTORS))
    else
        os.exit(1)
    end
end

return M
