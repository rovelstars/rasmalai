// matmul 256x256 integer matrices, ijk loop order.
const N = 256;
const a = new Array(N * N), b = new Array(N * N), c = new Array(N * N).fill(0);
for (let r = 0; r < N; r++)
    for (let k = 0; k < N; k++) {
        a[r * N + k] = (r + k) % 64;
        b[r * N + k] = (r * k) % 64;
    }
for (let x = 0; x < N; x++)
    for (let y = 0; y < N; y++) {
        let s = 0;
        for (let z = 0; z < N; z++) s += a[x * N + z] * b[z * N + y];
        c[x * N + y] = s;
    }
let total = 0;
for (let t = 0; t < N * N; t++) total += c[t];
console.log('RESULT checksum', total);
