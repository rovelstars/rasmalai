// mandelbrot 400x400, escape iterations capped at 1000.
void main() {
  var total = 0;
  var py = 0;
  while (py < 400) {
    final y0 = (py * 3.0 / 399.0) - 1.5;
    var px = 0;
    while (px < 400) {
      final x0 = (px * 3.0 / 399.0) - 2.0;
      var zx = 0.0;
      var zy = 0.0;
      var iter = 0;
      while (iter < 1000) {
        final zx2 = zx * zx;
        final zy2 = zy * zy;
        if (zx2 + zy2 > 4.0) break;
        zy = 2.0 * zx * zy + y0;
        zx = zx2 - zy2 + x0;
        iter++;
      }
      total += iter;
      px++;
    }
    py++;
  }
  print('RESULT checksum $total');
}
