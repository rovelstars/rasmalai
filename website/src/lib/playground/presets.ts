export interface Preset {
	name: string;
	source: string;
}

export const PRESETS: Preset[] = [
	{
		name: 'Hello world',
		source: `print("hello,", "world");`
	},
	{
		name: 'SIMD vector ops',
		source: `import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(2.0);
let c = a * b;
print(c.x(), c.y(), c.z(), c.w());
print(c.dot(b));`
	},
	{
		name: 'Resource cleanup',
		source: `defer { print("third"); }
defer { print("second"); }
print("first");`
	},
	{
		name: 'Pattern matching',
		source: `enum Shape { Circle(Float), Rect(Float, Float), Point }

fn area(s: Shape): Float {
    switch s {
        case .Circle(r): return 3.14 * r * r;
        case .Rect(w, h): return w * h;
        case .Point: return 0.0;
    }
}

print(area(Shape.Circle(2.0)));
print(area(Shape.Rect(3.0, 4.0)));`
	}
];
