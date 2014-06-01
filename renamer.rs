use std::vec::FromVec;
use module::*;
use scoped_map::ScopedMap;
use interner::*;

#[deriving(Eq, TotalEq, Hash, Clone)]
pub struct Name {
    pub name: InternedStr,
    pub uid: uint
}

impl Str for Name {
    fn as_slice<'a>(&'a self) -> &'a str {
        self.name.as_slice()
    }
}

impl ::std::fmt::Show for Name {
    fn fmt(&self, f: &mut ::std::fmt::Formatter) -> ::std::fmt::Result {
        write!(f, "{}_{}", self.name, self.uid)
    }
}

struct Errors<T> {
    errors: Vec<T>
}
impl <T> Errors<T> {
    fn new() -> Errors<T> {
        Errors { errors: Vec::new() }
    }
    fn insert(&mut self, e: T) {
        self.errors.push(e);
    }
    fn has_errors(&self) -> bool {
        self.errors.len() != 0
    }
}
impl <T: ::std::fmt::Show> Errors<T> {
    fn report_errors(&self, pass: &str) {
        println!("Found {} errors in compiler pass: {}", self.errors.len(), pass);
        for error in self.errors.iter() {
            println!("{}", error);
        }
    }
}

struct Renamer {
    uniques: ScopedMap<InternedStr, Name>,
    unique_id: uint,
    errors: Errors<StrBuf>
}

impl Renamer {
    fn new() -> Renamer {
        Renamer { uniques: ScopedMap::new(), unique_id: 1, errors: Errors::new() }
    }

    fn rename_bindings(&mut self, bindings: ~[Binding<InternedStr>], is_global: bool) -> ~[Binding<Name>] {
        //Add all bindings in the scope
        for bind in binding_groups(bindings.as_slice()) {
            self.make_unique(bind[0].name.clone());
            if is_global {
                self.uniques.find_mut(&bind[0].name).unwrap().uid = 0;
            }
        }
        FromVec::<Binding<Name>>::from_vec(bindings.move_iter().map(|binding| {
            let Binding { name: name, arguments: arguments, expression: expression, typeDecl: typeDecl, arity: arity  } = binding;
            let n = self.uniques.find(&name).map(|u| u.clone())
                .expect(format!("Error: lambda_lift: Undefined variable {}", name));
            self.uniques.enter_scope();
            let b = Binding {
                name: n,
                arguments: self.rename_arguments(arguments),
                expression: self.rename(expression),
                typeDecl: typeDecl,
                arity: arity
            };
            self.uniques.exit_scope();
            b
        }).collect())
    }

    fn rename(&mut self, input_expr: TypedExpr<InternedStr>) -> TypedExpr<Name> {
        let TypedExpr { expr: expr, typ: typ, location: location } = input_expr;
        let e = match expr {
            Literal(l) => Literal(l),
            Identifier(i) => Identifier(self.get_name(i)),
            Apply(func, arg) => Apply(box self.rename(*func), box self.rename(*arg)),
            Lambda(arg, body) => {
                self.uniques.enter_scope();
                let l = Lambda(self.rename_pattern(arg), box self.rename(*body));
                self.uniques.exit_scope();
                l
            }
            Let(bindings, expr) => {
                self.uniques.enter_scope();
                let bs = self.rename_bindings(bindings, false);
                let l = Let(bs, box self.rename(*expr));
                self.uniques.exit_scope();
                l
            }
            Case(expr, alts) => {
                let a: Vec<Alternative<Name>> = alts.move_iter().map(
                    |Alternative { pattern: Located { location: loc, node: pattern }, expression: expression }| {
                    self.uniques.enter_scope();
                    let a = Alternative {
                        pattern: Located { location: loc, node: self.rename_pattern(pattern) },
                        expression: self.rename(expression)
                    };
                    self.uniques.exit_scope();
                    a
                }).collect();
                Case(box self.rename(*expr), FromVec::from_vec(a))
            }
            Do(bindings, expr) => {
                let bs: Vec<DoBinding<Name>> = bindings.move_iter().map(|bind| {
                    match bind {
                        DoExpr(expr) => DoExpr(self.rename(expr)),
                        DoLet(bs) => DoLet(self.rename_bindings(bs, false)),
                        DoBind(pattern, expr) => {
                            let Located { location: location, node: node } = pattern;
                            let loc = Located { location: location, node: self.rename_pattern(node) };
                            DoBind(loc, self.rename(expr))
                        }
                    }
                }).collect();
                Do(FromVec::from_vec(bs), box self.rename(*expr))
            }
        };
        let mut t = TypedExpr::with_location(e, location);
        t.typ = typ;
        t
    }

    fn rename_pattern(&mut self, pattern: Pattern<InternedStr>) -> Pattern<Name> {
        match pattern {
            NumberPattern(i) => NumberPattern(i),
            ConstructorPattern(s, ps) => {
                let ps2: Vec<Pattern<Name>> = ps.move_iter().map(|p| self.rename_pattern(p)).collect();
                ConstructorPattern(Name { name: s, uid: 0}, FromVec::from_vec(ps2))
            }
            IdentifierPattern(s) => IdentifierPattern(self.make_unique(s)),
            WildCardPattern => WildCardPattern
        }
    }
    fn get_name(&self, s: InternedStr) -> Name {
        match self.uniques.find(&s) {
            Some(&Name { uid: uid, .. }) => Name { name: s, uid: uid },
            None => Name { name: s, uid: 0 }//If the variable is not found in variables it is a global variable
        }
    }

    fn rename_binding(&mut self, binding: Binding<InternedStr>) -> Binding<Name> {
        let Binding { name: name, arguments: arguments, expression: expression, typeDecl: td, arity: a } = binding;
        self.make_unique(name);
        self.uniques.find_mut(&name).unwrap().uid = 0;
        self.uniques.enter_scope();
        let b = Binding {
            name: Name { name: name, uid: 0 },
            arguments: self.rename_arguments(arguments),
            expression: self.rename(expression),
            typeDecl: td,
            arity: a
        };
        self.uniques.exit_scope();
        b
    }
    fn rename_arguments(&mut self, arguments: ~[Pattern<InternedStr>]) -> ~[Pattern<Name>] {
        FromVec::<Pattern<Name>>::from_vec(arguments.move_iter().map(|a| self.rename_pattern(a)).collect())
    }

    fn make_unique(&mut self, name: InternedStr) -> Name {
        if self.uniques.in_current_scope(&name) {
            self.errors.insert(format!("{} is defined multiple times", name));
            self.uniques.find(&name).map(|x| x.clone()).unwrap()
        }
        else {
            self.unique_id += 1;
            let u = Name { name: name.clone(), uid: self.unique_id};
            self.uniques.insert(name, u.clone());
            u
        }
    }
}

pub fn rename_expr(expr: TypedExpr<InternedStr>) -> TypedExpr<Name> {
    let mut renamer = Renamer::new();
    renamer.rename(expr)
}

pub fn rename_module(module: Module<InternedStr>) -> Module<Name> {
    let mut renamer = Renamer::new();
    rename_module_(&mut renamer, module)
}
pub fn rename_module_(renamer: &mut Renamer, module: Module<InternedStr>) -> Module<Name> {
    let Module {
        name: name,
        imports: imports,
        classes : classes,
        dataDefinitions: data_definitions,
        typeDeclarations: typeDeclarations,
        bindings : bindings,
        instances: instances
    } = module;

    let data_definitions2 : Vec<DataDefinition<Name>> = data_definitions.move_iter().map(|data| {
        let DataDefinition {
            constructors : ctors,
            typ : typ,
            parameters : parameters
        } = data;
        let c: Vec<Constructor<Name>> = ctors.move_iter().map(|ctor| {
            let Constructor {
                name : name,
                typ : typ,
                tag : tag,
                arity : arity
            } = ctor;
            Constructor {
                name : Name { name: name, uid: 0 },
                typ : typ,
                tag : tag,
                arity : arity
            }
        }).collect();

        DataDefinition {
            typ : typ,
            parameters : parameters,
            constructors : FromVec::from_vec(c)
        }
    }).collect();
    
    let instances2: Vec<Instance<Name>> = instances.move_iter().map(|instance| {
        let Instance {
            bindings : bindings,
            constraints : constraints,
            typ : typ,
            classname : classname
        } = instance;
        Instance {
            bindings : renamer.rename_bindings(bindings, true),
            constraints : constraints,
            typ : typ,
            classname : classname
        }
    }).collect();
    
    let bindings2 = renamer.rename_bindings(bindings, true);
    
    Module {
        name: renamer.make_unique(name),
        imports: imports,
        classes : classes,
        dataDefinitions: FromVec::from_vec(data_definitions2),
        typeDeclarations: typeDeclarations,
        bindings : bindings2,
        instances: FromVec::from_vec(instances2)
    }
}

pub fn rename_modules(modules: Vec<Module<InternedStr>>) -> Vec<Module<Name>> {
    let mut renamer = Renamer::new();
    let ms = modules.move_iter().map(|module| {
        rename_module_(&mut renamer, module)
    }).collect();
    if renamer.errors.has_errors() {
        renamer.errors.report_errors("Renamer");
        fail!();
    }
    ms
}

#[cfg(test)]
mod tests {
    use renamer::*;
    use parser::*;
    #[test]
    #[should_fail]
    fn duplicate_binding() {
        let mut parser = Parser::new(
r"main = 1
test = []
main = 2".chars());
        let module = parser.module();
        rename_modules(vec!(module));
    }
}
