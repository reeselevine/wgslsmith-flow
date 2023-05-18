use std::rc::Rc;

use serde_json::json;

use ast::types::{DataType, ScalarType, MemoryViewType};
use ast::{
    AccessMode, AssignmentLhs, AssignmentOp, AssignmentStatement, FnAttr, FnDecl, GlobalVarAttr,
    GlobalVarDecl, Module, ShaderStage, Statement, StorageClass, VarQualifier, ExprNode, Lit, 
    FnInput, FnInputAttr, VarDeclStatement, BinOp, VarExpr, BinOpExpr, Postfix, PostfixExpr, LetDeclStatement
};

use rand::distributions::{WeightedIndex};
use rand::prelude::{SliceRandom, StdRng};
use rand::Rng;
use rand_distr::{Distribution, Standard};

use crate::Options;

#[derive(PartialEq, Eq, Copy, Clone)]
enum AccessType {
  ThreadSafe,
  ThreadUnsafe,
  ThreadRace,
  ConstantSafe,
  ConstantUnsafe,
  VarSafe,
  VarUnsafe,
  Literal
}

impl Distribution<AccessType> for Standard {
  fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> AccessType {
      match rng.gen_range(0..100) {
          0..=12 => AccessType::ThreadSafe,
          13..=25 => AccessType::ThreadUnsafe,
          26..=38 => AccessType::ThreadRace,
          39..=51 => AccessType::ConstantSafe,
          52..=63 => AccessType::ConstantUnsafe,
          64..=75 => AccessType::VarSafe,
          76..=87 => AccessType::VarUnsafe,
          88..=99 => AccessType::Literal,
          _ => unreachable!(),
      }
  }
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum LhsAccessType {
  ThreadSafe,
  ThreadUnsafe,
  ThreadRace,
  ConstantUnsafe,
  VarSafe,
  VarUnsafe
}

impl Distribution<LhsAccessType> for Standard {
  fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> LhsAccessType {
      match rng.gen_range(0..100) {
          0..=16 => LhsAccessType::ThreadSafe,
          17..=33 => LhsAccessType::ThreadUnsafe,
          34..=50 => LhsAccessType::ThreadRace,
          51..=67 => LhsAccessType::ConstantUnsafe,
          68..=83 => LhsAccessType::VarSafe,
          84..=99 => LhsAccessType::VarUnsafe,
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
    safe_vars: Vec<String>,
    racy_vars: Vec<String>,
    lits: Vec<u32>,
    safe_offsets: Vec<u32>,
    racy_offsets: Vec<u32>,
    safe_constant_locs: Vec<u32>,
    racy_constant_locs: Vec<u32>,
    safe_access_types: Vec<AccessType>,
    safe_weights: WeightedIndex<i32>,
    lhs_access_types: Vec<AccessType>,
    lhs_weights: WeightedIndex<i32>
}

impl<'a> Generator<'a> {
    pub fn new(rng: &'a mut StdRng, options: Rc<Options>) -> Self {
        Generator {
            rng,
            options: options.clone(),
            safe_vars: vec![],
            racy_vars: vec![],
            lits: vec![],
            safe_offsets: vec![],
            racy_offsets: vec![],
            safe_constant_locs: vec![],
            racy_constant_locs: vec![],
            safe_access_types: vec![AccessType::ThreadSafe, AccessType::ConstantSafe, AccessType::VarSafe, AccessType::Literal],
            safe_weights: WeightedIndex::new(vec![30, 30, 20, 20]).unwrap(),
            lhs_access_types: vec![AccessType::ThreadSafe, AccessType::ThreadUnsafe, AccessType::ThreadRace, AccessType::ConstantUnsafe, AccessType::VarSafe, AccessType::VarUnsafe],
            lhs_weights: WeightedIndex::new(vec![17, 17, 17, 17, 16, 16]).unwrap()
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

        // Create literals used in the program 
        for i in 0..self.options.num_lits {
          self.lits.push(i);
        }

        // The first N memory locations are divided into safe (read-only) and racy (read-write)
        for i in 0..self.options.constant_locs {
          if self.rng_less_than(self.options.racy_constant_loc_pct) {
            self.racy_constant_locs.push(i);
          } else {
            self.safe_constant_locs.push(i);
          }
        }

        // Each thread's memory locations are divided into safe (read-write only by this thread)
        // and racy (read-write by any thread)
        for i in 0..self.options.locs_per_thread {
          if self.rng_less_than(self.options.racy_loc_pct) {
            self.racy_offsets.push(i);
          } else {
            self.safe_offsets.push(i);
          }
        }

        let mut block: Vec<Statement> = vec![];

        // total ids is the number of workgroups times the workgroup size
        block.push(LetDeclStatement::new(
          "total_ids",
          BinOpExpr::new(
            BinOp::Times, 
            VarExpr::new("num_workgroups.x").into_node(DataType::from(ScalarType::U32)), 
            ExprNode::from(Lit::U32(self.options.workgroup_size)))).into());

        // local variables are also divided into safe and racy
        for i in 0..self.options.vars {
            let name = format!("var_{i}");
            if self.rng_less_than(self.options.racy_var_pct) {
              self.racy_vars.push(name.to_owned());
            } else {
              self.safe_vars.push(name.to_owned());
            }
            block.push(self.initialize_var(name));
        }

        // Make a block to store statements that are safe
        let mut safe_block: Vec<Statement> = block.clone();

        for _i in 0..self.options.stmts {
          // Return a pair of AccessType + Statement
          // Then match, if safe add to both modules otherwise only to the race module
          let (stmt, access_type) = self.gen_statement();
          if self.safe_access_types.contains(&access_type) {
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

    fn constant_expr(&mut self, choices: Vec<u32>) -> ExprNode {
      ExprNode::from(Lit::U32(choices.choose(self.rng).unwrap().to_owned()))
    }

    fn gen_mem_idx(&mut self, access_type: AccessType) -> ExprNode {
      let offset = match access_type {
        AccessType::ThreadSafe => self.safe_offsets.choose(self.rng).unwrap().to_owned(),
        AccessType::ThreadUnsafe | AccessType::ThreadRace => self.racy_offsets.choose(self.rng).unwrap().to_owned(),
        _ => 0 //unused
      };
      match access_type {
        AccessType::ConstantSafe => {
          let choices = self.safe_constant_locs.to_owned();
          self.constant_expr(choices)
        },
        AccessType::ConstantUnsafe => {
          let choices = self.racy_constant_locs.to_owned();
          self.constant_expr(choices)
        },
        AccessType::ThreadSafe | AccessType::ThreadUnsafe => {
          let base_id = BinOpExpr::new(
            BinOp::Times,
            VarExpr::new("local_invocation_id.x").into_node(DataType::from(ScalarType::U32)),
            ExprNode::from(Lit::U32(self.options.locs_per_thread)));
          BinOpExpr::new(
            BinOp::Plus,
            base_id,
            ExprNode::from(Lit::U32(offset + self.options.constant_locs))).into()
        },
        AccessType::ThreadRace => {
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
        _ => unreachable!()
      }
    }

    fn gen_mem_access(&mut self, access_type: AccessType) -> ExprNode {
      let index = Postfix::index(self.gen_mem_idx(access_type));
      let arr_expr = VarExpr::new("mem").into_node(
        DataType::Ref(MemoryViewType::new(
    DataType::array(ScalarType::U32, None), StorageClass::Storage)));
      PostfixExpr::new(arr_expr, index).into()
    }

    fn var_expr(&mut self, choices: Vec<String>) -> ExprNode {
      VarExpr::new(choices.choose(self.rng).unwrap()).into_node(DataType::from(ScalarType::U32))
    }

    fn gen_op(&mut self, access_type: AccessType) -> ExprNode {
      match access_type {
        AccessType::Literal => ExprNode::from(Lit::U32(self.lits.choose(self.rng).unwrap().to_owned())),
        AccessType::VarSafe => {
          let choices = self.safe_vars.to_owned();
          self.var_expr(choices)
        },
        AccessType::VarUnsafe => {
          let choices = self.racy_vars.to_owned();
          self.var_expr(choices)
        },
        _ => self.gen_mem_access(access_type)
      }
    }

    fn gen_expr(&mut self, access_type: &AccessType) -> ExprNode {
      let left_access_type = if self.safe_access_types.contains(access_type) {
        self.safe_access_types[self.safe_weights.sample(self.rng)]
      } else {
        self.rng.gen()
      };

      let right_access_type = if self.safe_access_types.contains(access_type) {
        self.safe_access_types[self.safe_weights.sample(self.rng)]
      } else {
        self.rng.gen()
      };

      BinOpExpr::new(
        BinOp::Plus, 
        self.gen_op(left_access_type), 
        self.gen_op(right_access_type)).into()
    }

    fn gen_statement(&mut self) -> (Statement, AccessType) {
      let lhs_access_type: AccessType = self.lhs_access_types[self.lhs_weights.sample(self.rng)];
      let expr = self.gen_expr(&lhs_access_type.clone());
      let stmt = match lhs_access_type {
        AccessType::VarSafe => {
          let var = self.safe_vars.choose(self.rng).unwrap();
          AssignmentStatement::new(AssignmentLhs::name(var, DataType::from(ScalarType::U32)), AssignmentOp::Simple, expr).into()
        }
        AccessType::VarUnsafe => {
          let var = self.safe_vars.choose(self.rng).unwrap();
          AssignmentStatement::new(AssignmentLhs::name(var, DataType::from(ScalarType::U32)), AssignmentOp::Simple, expr).into()
        }
        _ => AssignmentStatement::new(
        AssignmentLhs::array_index(
          "mem", 
          DataType::Ref(MemoryViewType::new(
            DataType::array(ScalarType::U32, None), StorageClass::Storage)),
            self.gen_mem_idx(lhs_access_type)), 
        AssignmentOp::Simple,
        expr).into()
      };
      (stmt, lhs_access_type)
    }

    fn rng_less_than(&mut self, pct: u32) -> bool {
      return self.rng.gen_range(0..100) < pct;
    }
}
