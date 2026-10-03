// nbody: 5 bodies, classic Benchmarks-Game units, 100k steps, dt = 0.01.
import 'dart:math' as math;

const pi = 3.141592653589793;
const solar = 4.0 * pi * pi;
const days = 365.24;

void main() {
  final px = [0.0, 4.8414314424647209, 8.3433667182445799, 12.894369562139131, 15.3796971148509165];
  final py = [0.0, -1.1603200440274284, 4.1247985641243048, -15.1111514016986312, -25.9193146099879641];
  final pz = [0.0, -0.10362204447112311, -0.40352341711432138, -0.22330757889265573, 0.17925877295037118];
  final vx = [0.0, 0.0016600766427440369, -0.0027674251072686241, 0.0029646013756476162, 0.0026806777249038932];
  final vy = [0.0, 0.0076990111841974043, 0.0049985280123491724, 0.0023784717395948095, 0.001628241700382423];
  final vz = [0.0, -0.0000690460016972063, 0.00002304172975737639, -0.00002965895685402376, -0.00009515922545197159];
  final m = [solar, 0.00095479193842432661 * solar, 0.00028588598066613081 * solar,
             0.00004366244043351563 * solar, 0.00005151389302046611 * solar];
  for (var i = 0; i < 5; i++) {
    vx[i] *= days;
    vy[i] *= days;
    vz[i] *= days;
  }
  var ox = 0.0, oy = 0.0, oz = 0.0;
  for (var i = 1; i < 5; i++) {
    ox += vx[i] * m[i];
    oy += vy[i] * m[i];
    oz += vz[i] * m[i];
  }
  vx[0] = -ox / m[0];
  vy[0] = -oy / m[0];
  vz[0] = -oz / m[0];

  for (var s = 0; s < 100000; s++) {
    for (var i = 0; i < 5; i++) {
      for (var j = i + 1; j < 5; j++) {
        final dx = px[i] - px[j], dy = py[i] - py[j], dz = pz[i] - pz[j];
        final d2 = dx * dx + dy * dy + dz * dz;
        final mag = 0.01 / (d2 * math.sqrt(d2));
        vx[i] -= dx * m[j] * mag;
        vy[i] -= dy * m[j] * mag;
        vz[i] -= dz * m[j] * mag;
        vx[j] += dx * m[i] * mag;
        vy[j] += dy * m[i] * mag;
        vz[j] += dz * m[i] * mag;
      }
    }
    for (var k = 0; k < 5; k++) {
      px[k] += 0.01 * vx[k];
      py[k] += 0.01 * vy[k];
      pz[k] += 0.01 * vz[k];
    }
  }

  var e = 0.0;
  for (var i = 0; i < 5; i++) {
    e += 0.5 * m[i] * (vx[i] * vx[i] + vy[i] * vy[i] + vz[i] * vz[i]);
    for (var j = i + 1; j < 5; j++) {
      final dx = px[i] - px[j], dy = py[i] - py[j], dz = pz[i] - pz[j];
      e -= m[i] * m[j] / math.sqrt(dx * dx + dy * dy + dz * dz);
    }
  }
  print('RESULT checksum $e');
}
