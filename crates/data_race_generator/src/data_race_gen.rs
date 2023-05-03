use std::rc::Rc;

use ast::types::{DataType, ScalarType, MemoryViewType};
use ast::{
    AccessMode, AssignmentLhs, AssignmentOp, AssignmentStatement, FnAttr, FnDecl, GlobalVarAttr,
    GlobalVarDecl, Module, ShaderStage, Statement, StorageClass, VarQualifier, ExprNode, Lit, 
    FnInput, FnInputAttr, VarDeclStatement, BinOp, VarExpr, BinOpExpr, Postfix, PostfixExpr, LetDeclStatement, GlobalConstDecl
};
use rand::prelude::{SliceRandom, StdRng};
use rand::Rng;
use rand_distr::{Distribution, Standard};

use crate::Options;

// Enum for choosing operands in statements
enum OpType {
    Literal,
    LocalVar,
    Memory
}

impl Distribution<OpType> for Standard {
  fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> OpType {
      match rng.gen_range(0..3) {
          0 => OpType::Literal,
          1 => OpType::LocalVar,
          2 => OpType::Memory,
          _ => unreachable!(),
      }
  }
}

enum StmtType {
  Safe,
  DataRace,
  Constant
}

impl Distribution<StmtType> for Standard {
  fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> StmtType {
      match rng.gen_range(0..100) {
          0..=69 => StmtType::Safe,
          70..=89 => StmtType::DataRace,
          90..=99 => StmtType::Constant,
          _ => unreachable!(),
      }
  }
}


pub struct Generator<'a> {
    rng: &'a mut StdRng,
    options: Rc<Options>,
    vars: Vec<String>,
    lits: Vec<u32>
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut StdRng, options: Rc<Options>) -> Self {
        Generator {
            rng,
            options: options.clone(),
            vars: vec![],
            lits: vec![]
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn gen_module(&mut self) -> Module {
        let global_vars = vec![
            GlobalVarDecl {
                attrs: vec![GlobalVarAttr::Group(0), GlobalVarAttr::Binding(0)],
                qualifier: Some(VarQualifier {
                    storage_class: StorageClass::Storage,
                    access_mode: Some(AccessMode::ReadWrite),
                }),
                name: "mem".to_owned(),
                data_type: DataType::array(ScalarType::U32, None),
                initializer: None,
            }
        ];

        let global_consts = vec![
          GlobalConstDecl {
            name: "workgroup_size".to_owned(),
            data_type: DataType::from(ScalarType::U32),
            initializer: ExprNode::from(Lit::U32(self.options.workgroup_size))
          }
        ];

        for i in 0..self.options.num_lits {
          self.lits.push(i);
        }

        let mut block: Vec<Statement> = vec![];

        block.push(LetDeclStatement::new(
          "total_ids",
          BinOpExpr::new(
            BinOp::Times, 
            VarExpr::new("num_workgroups.x").into_node(DataType::from(ScalarType::U32)), 
            VarExpr::new("workgroup_size").into_node(DataType::from(ScalarType::U32)))).into());

        let var_count = self.rng.gen_range(self.options.min_vars..=self.options.max_vars);

        for i in 0..var_count {
            let name = format!("var_{i}");
            self.vars.push(name.clone());
            block.push(self.initialize_var(name));
        }

        let statement_count = self.rng.gen_range(self.options.min_stmts..=self.options.max_stmts);
        for _i in 0..statement_count {
          block.push(self.gen_statement());
        }

        let mut num_workgroups = FnInput::new("num_workgroups", DataType::Vector(3, ScalarType::U32));
        num_workgroups.attrs.push(FnInputAttr::Builtin("num_workgroups".to_string()));
        let mut local_invocation_id = FnInput::new("local_invocation_id", DataType::Vector(3, ScalarType::U32));
        local_invocation_id.attrs.push(FnInputAttr::Builtin("local_invocation_id".to_string()));
        let mut workgroup_id = FnInput::new("workgroup_id", DataType::Vector(3, ScalarType::U32));
        workgroup_id.attrs.push(FnInputAttr::Builtin("workgroup_id".to_string()));

        let entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::VarWorkgroupSize("workgroup_size".to_owned()),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups, workgroup_id, local_invocation_id],
            output: None,
            body: block,
        };

        let functions = vec![entrypoint];

        Module {
            structs: vec![],
            consts: global_consts, 
            vars: global_vars,
            functions
        }
    }

    fn initialize_var(&mut self, name: String) -> Statement {
      let ty = ScalarType::U32;
      VarDeclStatement::new(name, Some(ty.into()), None).into()
    }

    fn gen_mem_idx(&mut self) -> ExprNode {
      let base_id = BinOpExpr::new(
        BinOp::Times,
        VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
        ExprNode::from(Lit::U32(self.options.locs_per_thread)));
      let offset = self.rng.gen_range(0..self.options.locs_per_thread);
      BinOpExpr::new(
        BinOp::Plus,
        base_id,
        ExprNode::from(Lit::U32(offset))).into()
    }

    fn gen_mem_access(&mut self) -> ExprNode {
      let index = Postfix::index(self.gen_mem_idx());
      let arr_expr = VarExpr::new("mem").into_node(
        DataType::Ref(MemoryViewType::new(
    DataType::array(ScalarType::U32, None), StorageClass::Storage)));
      PostfixExpr::new(arr_expr, index).into()
    }

    fn gen_op(&mut self) -> ExprNode {
       match self.rng.gen() {
        OpType::Literal => ExprNode::from(Lit::U32(self.lits.choose(self.rng).unwrap().clone())),
        OpType::LocalVar => VarExpr::new(self.vars.choose(self.rng).unwrap()).into_node(DataType::from(ScalarType::U32)),
        OpType::Memory => self.gen_mem_access()
      }
    }

    fn gen_expr(&mut self) -> ExprNode {
      BinOpExpr::new(BinOp::Plus, self.gen_op(), self.gen_op()).into()
    }

    fn gen_statement(&mut self) -> Statement {
      let expr = self.gen_expr();
      AssignmentStatement::new(
        AssignmentLhs::array_index(
          "mem", 
          DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None), StorageClass::Storage)),
            self.gen_mem_idx()), 
        AssignmentOp::Simple,
        expr).into()
    }
}
