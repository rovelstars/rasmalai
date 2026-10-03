// spectral-norm N=300, 10 power iterations on AtA.
import 'dart:math' as math;

const n = 300;

double aElem(int i, int j) {
  final w = (i + j) * (i + j + 1) ~/ 2 + i + 1;
  return 1.0 / w;
}

void av(List<double> x, List<double> y) {
  for (var i = 0; i < n; i++) {
    var s = 0.0;
    for (var j = 0; j < n; j++) {
      s += aElem(i, j) * x[j];
    }
    y[i] = s;
  }
}

void atv(List<double> x, List<double> y) {
  for (var i = 0; i < n; i++) {
    var s = 0.0;
    for (var j = 0; j < n; j++) {
      s += aElem(j, i) * x[j];
    }
    y[i] = s;
  }
}

void main() {
  final u = List<double>.filled(n, 1.0);
  final v = List<double>.filled(n, 0.0);
  final tmp = List<double>.filled(n, 0.0);
  for (var k = 0; k < 10; k++) {
    av(u, tmp);
    atv(tmp, v);
    av(v, tmp);
    atv(tmp, u);
  }
  var vBv = 0.0, vv = 0.0;
  for (var j = 0; j < n; j++) {
    vBv += u[j] * v[j];
    vv += v[j] * v[j];
  }
  print('RESULT checksum ${math.sqrt(vBv / vv)}');
}
