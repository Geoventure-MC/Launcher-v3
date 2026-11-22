/**
 * BLAKE2b-512 en JavaScript pur (RFC 7693), pour vérifier les signatures minisign quand le module
 * crypto d'Electron (BoringSSL) ne propose pas cet algorithme. Port compact de blakejs (domaine public / CC0).
 */
'use strict';

const IV32 = new Uint32Array([
    0xF3BCC908, 0x6A09E667, 0x84CAA73B, 0xBB67AE85, 0xFE94F82B, 0x3C6EF372, 0x5F1D36F1, 0xA54FF53A,
    0xADE682D1, 0x510E527F, 0x2B3E6C1F, 0x9B05688C, 0xFB41BD6B, 0x1F83D9AB, 0x137E2179, 0x5BE0CD19,
]);
const SIGMA8 = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
    11, 8, 12, 0, 5, 2, 15, 13, 10, 14, 3, 6, 7, 1, 9, 4, 7, 9, 3, 1, 13, 12, 11, 14, 2, 6, 5, 10, 4, 0, 15, 8,
    9, 0, 5, 7, 2, 4, 10, 15, 14, 1, 11, 12, 6, 8, 3, 13, 2, 12, 6, 10, 0, 11, 8, 3, 4, 13, 7, 5, 15, 14, 1, 9,
    12, 5, 1, 15, 14, 13, 4, 10, 0, 7, 6, 3, 9, 2, 8, 11, 13, 11, 7, 14, 12, 1, 3, 9, 5, 0, 15, 4, 8, 6, 2, 10,
    6, 15, 14, 9, 11, 3, 0, 8, 12, 2, 13, 7, 1, 4, 10, 5, 10, 2, 8, 4, 7, 6, 1, 5, 15, 11, 9, 14, 3, 12, 13, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 14, 10, 4, 8, 9, 15, 13, 6, 1, 12, 0, 2, 11, 7, 5, 3,
];
const SIGMA82 = new Uint8Array(SIGMA8.map((x) => x * 2));
const v = new Uint32Array(32);
const m = new Uint32Array(32);

function ADD64AA(a, i, j) {
    const o0 = a[i] + a[j];
    let o1 = a[i + 1] + a[j + 1];
    if (o0 >= 0x100000000) o1++;
    a[i] = o0; a[i + 1] = o1;
}
function ADD64AC(a, i, b0, b1) {
    let o0 = a[i] + b0;
    if (b0 < 0) o0 += 0x100000000;
    let o1 = a[i + 1] + b1;
    if (o0 >= 0x100000000) o1++;
    a[i] = o0; a[i + 1] = o1;
}
function G(a, b, c, d, ix, iy) {
    const x0 = m[ix], x1 = m[ix + 1], y0 = m[iy], y1 = m[iy + 1];
    ADD64AA(v, a, b); ADD64AC(v, a, x0, x1);
    let xor0 = v[d] ^ v[a], xor1 = v[d + 1] ^ v[a + 1];
    v[d] = xor1; v[d + 1] = xor0;
    ADD64AA(v, c, d);
    xor0 = v[b] ^ v[c]; xor1 = v[b + 1] ^ v[c + 1];
    v[b] = (xor0 >>> 24) ^ (xor1 << 8); v[b + 1] = (xor1 >>> 24) ^ (xor0 << 8);
    ADD64AA(v, a, b); ADD64AC(v, a, y0, y1);
    xor0 = v[d] ^ v[a]; xor1 = v[d + 1] ^ v[a + 1];
    v[d] = (xor0 >>> 16) ^ (xor1 << 16); v[d + 1] = (xor1 >>> 16) ^ (xor0 << 16);
    ADD64AA(v, c, d);
    xor0 = v[b] ^ v[c]; xor1 = v[b + 1] ^ v[c + 1];
    v[b] = (xor1 >>> 31) ^ (xor0 << 1); v[b + 1] = (xor0 >>> 31) ^ (xor1 << 1);
}
function compress(ctx, last) {
    for (let i = 0; i < 16; i++) { v[i] = ctx.h[i]; v[i + 16] = IV32[i]; }
    v[24] ^= ctx.t; v[25] ^= ctx.t / 0x100000000;
    if (last) { v[28] = ~v[28]; v[29] = ~v[29]; }
    for (let i = 0; i < 32; i++) {
        const o = 4 * i;
        m[i] = ctx.b[o] ^ (ctx.b[o + 1] << 8) ^ (ctx.b[o + 2] << 16) ^ (ctx.b[o + 3] << 24);
    }
    for (let i = 0; i < 12; i++) {
        const s = i * 16;
        G(0, 8, 16, 24, SIGMA82[s], SIGMA82[s + 1]);
        G(2, 10, 18, 26, SIGMA82[s + 2], SIGMA82[s + 3]);
        G(4, 12, 20, 28, SIGMA82[s + 4], SIGMA82[s + 5]);
        G(6, 14, 22, 30, SIGMA82[s + 6], SIGMA82[s + 7]);
        G(0, 10, 20, 30, SIGMA82[s + 8], SIGMA82[s + 9]);
        G(2, 12, 22, 24, SIGMA82[s + 10], SIGMA82[s + 11]);
        G(4, 14, 16, 26, SIGMA82[s + 12], SIGMA82[s + 13]);
        G(6, 8, 18, 28, SIGMA82[s + 14], SIGMA82[s + 15]);
    }
    for (let i = 0; i < 16; i++) ctx.h[i] = ctx.h[i] ^ v[i] ^ v[i + 16];
}

/** Empreinte BLAKE2b-512 (64 octets) d'un Buffer ou Uint8Array. */
function blake2b512(input) {
    const ctx = { b: new Uint8Array(128), h: new Uint32Array(16), t: 0, c: 0 };
    for (let i = 0; i < 16; i++) ctx.h[i] = IV32[i];
    ctx.h[0] ^= 0x01010000 ^ 64;
    for (let i = 0; i < input.length; i++) {
        if (ctx.c === 128) { ctx.t += ctx.c; compress(ctx, false); ctx.c = 0; }
        ctx.b[ctx.c++] = input[i];
    }
    ctx.t += ctx.c;
    while (ctx.c < 128) ctx.b[ctx.c++] = 0;
    compress(ctx, true);
    const out = Buffer.alloc(64);
    for (let i = 0; i < 64; i++) out[i] = ctx.h[i >> 2] >> (8 * (i & 3));
    return out;
}

module.exports = { blake2b512 };
