use std::rc::Rc;

use serde_json::json;

use ast::types::{DataType, ScalarType, MemoryViewType};
use ast::{
    AccessMode, AssignmentLhs, AssignmentOp, AssignmentStatement, FnAttr, FnDecl, GlobalVarAttr,
    GlobalVarDecl, Module, ShaderStage, Statement, StorageClass, VarQualifier, ExprNode, Lit, 
    FnInput, FnInputAttr, VarDeclStatement, BinOp, VarExpr, BinOpExpr, Postfix, PostfixExpr, LetDeclStatement
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

#[derive(PartialEq, Eq, Copy, Clone)]
enum AccessType {
  Safe,
  Unsafe,
  DataRace,
  Constant
}

impl Distribution<AccessType> for Standard {
  fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AccessType {
      match rng.gen_range(0..100) {
          0..=59 => AccessType::Safe,
          60..=79 => AccessType::Unsafe,
          80..=89 => AccessType::DataRace,
          90..=99 => AccessType::Constant,
          _ => unreachable!(),
      }
  }
}

pub struct Outputs {
    pub safe: Module,
    pub race: Module,
    pub info: serde_json::Value
}

pub struct Generator<'a> {
    rng: &'a mut StdRng,
    options: Rc<Options>,
    vars: Vec<String>,
    lits: Vec<u32>,
    safe_offsets: Vec<u32>,
    race_offsets: Vec<u32> 
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut StdRng, options: Rc<Options>) -> Self {
        Generator {
            rng,
            options: options.clone(),
            vars: vec![],
            lits: vec![],
            safe_offsets: vec![],
            race_offsets: vec![]
        }
    }

    #[tracing::instrument(skip(self))]
    pub fn gen_module(&mut self) -> Outputs {
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

        for i in 0..self.options.num_lits {
          self.lits.push(i);
        }

        for i in 0..self.options.locs_per_thread {
          if self.rng.gen_range(0..100) < self.options.racy_loc_pct {
            self.race_offsets.push(i);
          } else {
            self.safe_offsets.push(i);
          }
        }

        let mut block: Vec<Statement> = vec![];

        block.push(LetDeclStatement::new(
          "total_ids",
          BinOpExpr::new(
            BinOp::Times, 
            VarExpr::new("num_workgroups.x").into_node(DataType::from(ScalarType::U32)), 
            ExprNode::from(Lit::U32(self.options.workgroup_size)))).into());

        let var_count = self.rng.gen_range(self.options.min_vars..=self.options.max_vars);

        for i in 0..var_count {
            let name = format!("var_{i}");
            self.vars.push(name.to_owned());
            block.push(self.initialize_var(name));
        }

        // Make a block to store statements that are safe
        let mut safe_block: Vec<Statement> = block.clone();

        let statement_count = self.rng.gen_range(self.options.min_stmts..=self.options.max_stmts);
        for _i in 0..statement_count {
          // Return a pair of AccessType + Statement
          // Then match, if safe add to both modules otherwise only to the race module
          let (stmt, access_type) = self.gen_statement();
          println!("stmt: {:?}", stmt);
          if access_type == AccessType::Safe {
            safe_block.push(stmt.clone());
          }
          block.push(stmt);
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
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups.clone(), workgroup_id.clone(), local_invocation_id.clone()],
            output: None,
            body: block,
        };

        let safe_entrypoint = FnDecl {
            attrs: vec![
                FnAttr::Stage(ShaderStage::Compute),
                FnAttr::LitWorkgroupSize(self.options.workgroup_size),
            ],
            name: "main".to_owned(),
            inputs: vec![num_workgroups, workgroup_id, local_invocation_id],
            output: None,
            body: safe_block,
        };

        let functions = vec![entrypoint];
        let safe_functions = vec![safe_entrypoint];

        // Write the safe values to a json file to be read later
        let json_info = json!({
            "safe": self.safe_offsets
        });
        
        Outputs {
            safe: Module {
                    structs: vec![],
                    consts: vec![], 
                    vars: global_vars.clone(),
                    functions: safe_functions
                },
            race: Module {
                    structs: vec![],
                    consts: vec![],
                    vars: global_vars.clone(),
                    functions
                },
            info: json_info
        } 
    }

    fn initialize_var(&mut self, name: String) -> Statement {
      let ty = ScalarType::U32;
      VarDeclStatement::new(name, Some(ty.into()), None).into()
    }

    fn gen_mem_idx(&mut self, access_type: AccessType) -> ExprNode {
      let offset = match access_type {
        AccessType::Safe => self.safe_offsets.choose(self.rng).unwrap().to_owned(),
        AccessType::Unsafe | AccessType::DataRace => self.race_offsets.choose(self.rng).unwrap().to_owned(),
        _ => 0 //unused
      };
      match access_type {
        AccessType::Constant => ExprNode::from(Lit::U32(self.rng.gen_range(0..self.options.locs_per_thread))),
        AccessType::Safe | AccessType::Unsafe => {
          let base_id = BinOpExpr::new(
            BinOp::Times,
            VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
            ExprNode::from(Lit::U32(self.options.locs_per_thread)));
          BinOpExpr::new(
            BinOp::Plus,
            base_id,
            ExprNode::from(Lit::U32(offset))).into()
        },
        AccessType::DataRace => {
          let new_id = BinOpExpr::new(
            BinOp::Plus,
            VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
            ExprNode::from(Lit::U32(self.rng.gen_range(1..1024)))); //TODO: magic number

          let mod_id = BinOpExpr::new(
            BinOp::Mod,
            new_id,
            VarExpr::new("total_ids").into_node(DataType::from(ScalarType::U32)));

          let base_id = BinOpExpr::new(
            BinOp::Times,
            mod_id,
            ExprNode::from(Lit::U32(self.options.locs_per_thread)));
          BinOpExpr::new(
            BinOp::Plus,
            base_id,
            ExprNode::from(Lit::U32(offset))).into()
        }
      }
    }

    fn gen_mem_access(&mut self, access_type: AccessType) -> ExprNode {
      let index = Postfix::index(self.gen_mem_idx(access_type));
      let arr_expr = VarExpr::new("mem").into_node(
        DataType::Ref(MemoryViewType::new(
    DataType::array(ScalarType::U32, None), StorageClass::Storage)));
      PostfixExpr::new(arr_expr, index).into()
    }

    fn gen_op(&mut self, access_type: AccessType) -> ExprNode {
       match self.rng.gen() {
        OpType::Literal => ExprNode::from(Lit::U32(self.lits.choose(self.rng).unwrap().to_owned())),
        OpType::LocalVar => VarExpr::new(self.vars.choose(self.rng).unwrap()).into_node(DataType::from(ScalarType::U32)),
        OpType::Memory => self.gen_mem_access(access_type)
      }
    }

    fn gen_expr(&mut self, access_type: &AccessType) -> ExprNode {
      let left_access_ty: AccessType = match access_type {
        AccessType::Safe => AccessType::Safe,
        _ => self.rng.gen()
      };
      let right_access_ty: AccessType = match access_type {
        AccessType::Safe => AccessType::Safe,
        _ => self.rng.gen()
      };
      BinOpExpr::new(
        BinOp::Plus, 
        self.gen_op(left_access_ty), 
        self.gen_op(right_access_ty)).into()
    }

    fn gen_statement(&mut self) -> (Statement, AccessType) {
      let access_type: AccessType = self.rng.gen();
      let expr = self.gen_expr(&access_type.clone());
      (AssignmentStatement::new(
        AssignmentLhs::array_index(
          "mem", 
          DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None), StorageClass::Storage)),
            self.gen_mem_idx(access_type)), 
        AssignmentOp::Simple,
        expr).into(), access_type)
    }
}
