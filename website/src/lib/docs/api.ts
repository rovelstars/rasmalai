export interface JsDocTag {
	kind: string;
	name: string;
	text: string;
}

export interface JsDoc {
	description: string;
	tags: JsDocTag[];
}

export interface DocFn {
	name: string;
	sig: string;
	docs: JsDoc;
}

export interface DocField {
	name: string;
	ty: string;
	docs: JsDoc;
}

export interface DocMethod {
	name: string;
	sig: string;
	docs: JsDoc;
}

export interface DocClass {
	name: string;
	docs: JsDoc;
	init: string | null;
	fields: DocField[];
	methods: DocMethod[];
}

export interface DocVariant {
	name: string;
	payload: string[];
	docs: JsDoc;
}

export interface DocEnum {
	name: string;
	docs: JsDoc;
	variants: DocVariant[];
}

export interface DocConst {
	name: string;
	ty: string;
	docs: JsDoc;
}

export interface DocModule {
	name: string;
	docs: JsDoc;
	functions: DocFn[];
	classes: DocClass[];
	enums: DocEnum[];
	constants: DocConst[];
}
