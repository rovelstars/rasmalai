// matmul 256x256 integer matrices, ijk loop order.
const n = 256;

void main() {
  final a = List<int>.filled(n * n, 0);
  final b = List<int>.filled(n * n, 0);
  final c = List<int>.filled(n * n, 0);
  for (var r = 0; r < n; r++) {
    for (var k = 0; k < n; k++) {
      a[r * n + k] = (r + k) % 64;
      b[r * n + k] = (r * k) % 64;
    }
  }
  for (var x = 0; x < n; x++) {
    for (var y = 0; y < n; y++) {
      var s = 0;
      for (var z = 0; z < n; z++) {
        s += a[x * n + z] * b[z * n + y];
      }
      c[x * n + y] = s;
    }
  }
  var total = 0;
  for (var t = 0; t < n * n; t++) {
    total += c[t];
  }
  print('RESULT checksum $total');
}
