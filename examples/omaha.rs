use ducy::deck::{Card, Deck};
use ducy::games::flop_game::FlopGame;
use ducy::games::omaha::{OmahaBoardAnalysis, OmahaGameEvaluation, OmahaGameState};
use ducy::games::{GameEquityEvaluation, GameEvaluation};

fn main() {
    let mut game = OmahaGameState::new(4);
    game.add_player(Deck::parse("As Ac Jc Ts").unwrap())
        .unwrap();
    game.add_player(Deck::parse("9h 8h 7d 6d").unwrap())
        .unwrap();

    game.set_flop(Deck::parse("Jh Th Qd").unwrap()).unwrap();
    println!("Board tone: {:?}", game.board_tone());

    let evaluator = OmahaGameEvaluation {};

    let winners = evaluator.evaluate_winners(&game);
    println!("\nAfter flop (Jh Th Qd):");
    for w in &winners {
        println!("  Player {} wins with {}", w.player_index, w.winning_hand);
    }

    game.set_turn(Card::parse("Jd").unwrap()).unwrap();

    let winners = evaluator.evaluate_winners(&game);
    println!("\nAfter turn (Jd):");
    for w in &winners {
        println!("  Player {} wins with {}", w.player_index, w.winning_hand);
    }

    let equity = evaluator.evaluate_equity(&game);
    println!("\nEquity on the turn:");
    for (i, eq) in equity.iter().enumerate() {
        println!("  Player {i}: {eq}");
    }

    game.set_river(Card::parse("Qh").unwrap()).unwrap();
    println!("\nBoard tone after river: {:?}", game.board_tone());

    let winners = evaluator.evaluate_winners(&game);
    println!("\nAfter river (Qh):");
    for w in &winners {
        println!("  Player {} wins with {}", w.player_index, w.winning_hand);
    }
}
